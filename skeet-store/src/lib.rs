#![warn(clippy::all, clippy::nursery)]
mod error;
mod schema;
mod types;

pub use error::StoreError;
pub use shared::ConfigVersion;
pub use types::{DiscoveredAt, ImageId, ImageRecord, InvalidImageId, OriginalAt, SkeetId, Zone};

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{
    Array, Int64Array, LargeBinaryArray, RecordBatch, RecordBatchIterator, StringArray,
    TimestampMicrosecondArray,
};
use chrono::{DateTime, TimeZone, Utc};
use futures::TryStreamExt;
use image::DynamicImage;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::OptimizeAction;
use tracing::{info, instrument};

use schema::{TABLE_NAME, VALIDATE_TABLE_NAME, images_v6_schema, validate_v1_schema};

#[derive(Clone, Debug, clap::Args)]
pub struct StoreArgs {
    /// Store location: local path or S3 URI (e.g. s3://bucket/path)
    #[arg(long)]
    pub store_path: String,

    /// S3-compatible endpoint URL (e.g. https://<account>.r2.cloudflarestorage.com)
    #[arg(long, env = "BOBBY_S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    /// S3 access key ID
    #[arg(long, env = "BOBBY_S3_ACCESS_KEY_ID")]
    pub s3_access_key_id: Option<String>,

    /// S3 secret access key
    #[arg(long, env = "BOBBY_S3_SECRET_ACCESS_KEY")]
    pub s3_secret_access_key: Option<String>,

    /// S3 region (default: auto, suitable for Cloudflare R2)
    #[arg(long, default_value = "auto")]
    pub s3_region: String,

    /// SSE-C encryption key (base64-encoded 256-bit AES key); enables server-side encryption
    #[arg(long, env = "BOBBY_SSE_C_KEY")]
    pub sse_c_key: Option<String>,
}

impl StoreArgs {
    pub fn storage_options(&self) -> Vec<(String, String)> {
        let mut opts = Vec::new();
        if let Some(endpoint) = &self.s3_endpoint {
            opts.push(("aws_endpoint".into(), endpoint.clone()));
        }
        if let Some(key_id) = &self.s3_access_key_id {
            opts.push(("aws_access_key_id".into(), key_id.clone()));
        }
        if let Some(secret) = &self.s3_secret_access_key {
            opts.push(("aws_secret_access_key".into(), secret.clone()));
        }
        opts.push(("aws_region".into(), self.s3_region.clone()));
        opts.push(("client_max_retries".into(), "0".into()));
        opts.push(("client_retry_timeout".into(), "30".into()));
        if let Some(key) = &self.sse_c_key {
            opts.push(("aws_server_side_encryption".into(), "sse-c".into()));
            opts.push(("aws_sse_customer_key_base64".into(), key.clone()));
        }
        opts
    }

    #[instrument(skip(self), fields(store_path = %self.store_path))]
    pub async fn open_store(&self) -> Result<SkeetStore, StoreError> {
        SkeetStore::open(&self.store_path, self.storage_options()).await
    }
}

pub struct SkeetStore {
    db: lancedb::Connection,
}

impl SkeetStore {
    #[instrument(skip(storage_options))]
    pub async fn open(
        uri: &str,
        storage_options: Vec<(String, String)>,
    ) -> Result<Self, StoreError> {
        info!(uri, "opening store");
        let db = lancedb::connect(uri)
            .storage_options(storage_options)
            .execute()
            .await?;

        let table_names = db.table_names().execute().await?;
        if !table_names.contains(&TABLE_NAME.to_string()) {
            db.create_empty_table(TABLE_NAME, images_v6_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&VALIDATE_TABLE_NAME.to_string()) {
            db.create_empty_table(VALIDATE_TABLE_NAME, validate_v1_schema())
                .execute()
                .await?;
        }

        let table = db.open_table(TABLE_NAME).execute().await?;
        let indices = table.list_indices().await?;
        if !indices.iter().any(|idx| idx.columns == vec!["image_id"]) {
            table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }

        info!(uri, "store opened");
        Ok(Self { db })
    }

    #[instrument(skip(self, record), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
    pub async fn add(&self, record: &ImageRecord) -> Result<(), StoreError> {
        let schema = images_v6_schema();

        let image_bytes = encode_image_as_png(&record.image)?;
        let annotated_bytes = encode_image_as_png(&record.annotated_image)?;
        let image_id_str = record.image_id.to_string();
        let skeet_id_str = record.skeet_id.to_string();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![image_id_str.as_str()])),
                Arc::new(StringArray::from(vec![skeet_id_str.as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&image_bytes])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.discovered_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.original_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(StringArray::from(vec![record.zone.to_string().as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&annotated_bytes])),
                Arc::new(StringArray::from(vec![record.config_version.as_str()])),
                Arc::new(StringArray::from(vec![record.detected_text.as_str()])),
            ],
        )?;

        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(batches).execute().await?;
        table.optimize(OptimizeAction::All).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn list_all(&self) -> Result<Vec<StoredImage>, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches: Vec<RecordBatch> = table.query().execute().await?.try_collect().await?;
        batches_to_stored_images(&batches)
    }

    #[instrument(skip(self))]
    pub async fn list_all_summaries(&self) -> Result<Vec<StoredImageSummary>, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches: Vec<RecordBatch> = table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "skeet_id",
                "discovered_at",
                "original_at",
                "archetype",
                "config_version",
                "detected_text",
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;
        batches_to_summaries(&batches)
    }

    #[instrument(skip(self))]
    pub async fn get_by_id(&self, image_id: &ImageId) -> Result<Option<StoredImage>, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches: Vec<RecordBatch> = table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;
        Ok(batches_to_stored_images(&batches)?.into_iter().next())
    }

    #[instrument(skip(self))]
    pub async fn exists(&self, image_id: &ImageId) -> Result<bool, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches: Vec<RecordBatch> = table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .select(lancedb::query::Select::columns(&["image_id"]))
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;
        Ok(batches.iter().any(|b| b.num_rows() > 0))
    }

    #[instrument(skip(self))]
    pub async fn count(&self) -> Result<usize, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        Ok(table.count_rows(None).await?)
    }

    #[instrument(skip(self))]
    pub async fn validate(&self) -> Result<(), StoreError> {
        let now = Utc::now();
        let timestamp_micros = now.timestamp_micros();
        let random_number = rand::random::<i64>();

        let schema = validate_v1_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(
                    TimestampMicrosecondArray::from(vec![timestamp_micros]).with_timezone("UTC"),
                ),
                Arc::new(Int64Array::from(vec![random_number])),
            ],
        )?;

        let table = self.db.open_table(VALIDATE_TABLE_NAME).execute().await?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(batches).execute().await?;

        let result_batches: Vec<RecordBatch> = table
            .query()
            .only_if(format!("random_number = {random_number}"))
            .execute()
            .await?
            .try_collect()
            .await?;

        if result_batches.is_empty() {
            return Err(StoreError::ValidationFailed(
                "no rows returned for validation query".to_string(),
            ));
        }

        let timestamps =
            typed_column::<TimestampMicrosecondArray>(&result_batches[0], "timestamp")?;
        if result_batches[0].num_rows() == 0 {
            return Err(StoreError::ValidationFailed(
                "no rows returned for validation query".to_string(),
            ));
        }

        let found_micros = timestamps.value(0);
        if found_micros != timestamp_micros {
            return Err(StoreError::ValidationFailed(format!(
                "timestamp mismatch: expected {timestamp_micros}, got {found_micros}"
            )));
        }

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn unique_skeet_ids(&self) -> Result<Vec<SkeetId>, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches: Vec<RecordBatch> = table
            .query()
            .select(lancedb::query::Select::columns(&["skeet_id"]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for batch in &batches {
            let skeet_ids = typed_column::<StringArray>(batch, "skeet_id")?;
            for i in 0..batch.num_rows() {
                let id = skeet_ids.value(i).to_string();
                if seen.insert(id.clone()) {
                    ids.push(SkeetId::new(id)?);
                }
            }
        }

        Ok(ids)
    }
}

pub struct StoredImage {
    pub summary: StoredImageSummary,
    pub image: DynamicImage,
    pub annotated_image: DynamicImage,
}

impl From<StoredImage> for ImageRecord {
    fn from(stored: StoredImage) -> Self {
        Self {
            image_id: stored.summary.image_id,
            skeet_id: stored.summary.skeet_id,
            image: stored.image,
            discovered_at: stored.summary.discovered_at,
            original_at: stored.summary.original_at,
            zone: stored.summary.zone,
            annotated_image: stored.annotated_image,
            config_version: stored.summary.config_version,
            detected_text: stored.summary.detected_text,
        }
    }
}

pub struct StoredImageSummary {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
    pub zone: Zone,
    pub config_version: ConfigVersion,
    pub detected_text: String,
}

struct SummaryColumns<'a> {
    image_ids: &'a StringArray,
    skeet_ids: &'a StringArray,
    discovered_ats: &'a TimestampMicrosecondArray,
    original_ats: &'a TimestampMicrosecondArray,
    archetypes: &'a StringArray,
    config_versions: &'a StringArray,
    detected_texts: &'a StringArray,
}

impl<'a> SummaryColumns<'a> {
    #[instrument(skip(batch))]
    fn extract(batch: &'a RecordBatch) -> Result<Self, StoreError> {
        Ok(Self {
            image_ids: typed_column::<StringArray>(batch, "image_id")?,
            skeet_ids: typed_column::<StringArray>(batch, "skeet_id")?,
            discovered_ats: typed_column::<TimestampMicrosecondArray>(batch, "discovered_at")?,
            original_ats: typed_column::<TimestampMicrosecondArray>(batch, "original_at")?,
            archetypes: typed_column::<StringArray>(batch, "archetype")?,
            config_versions: typed_column::<StringArray>(batch, "config_version")?,
            detected_texts: typed_column::<StringArray>(batch, "detected_text")?,
        })
    }

    #[instrument(skip(self))]
    fn to_summary(&self, i: usize) -> Result<StoredImageSummary, StoreError> {
        let zone: Zone = self
            .archetypes
            .value(i)
            .parse()
            .map_err(|_| StoreError::InvalidArchetype(self.archetypes.value(i).to_string()))?;
        let config_version: ConfigVersion = self
            .config_versions
            .value(i)
            .parse()
            .expect("ConfigVersion parse is infallible");
        Ok(StoredImageSummary {
            image_id: self.image_ids.value(i).parse()?,
            skeet_id: SkeetId::new(self.skeet_ids.value(i))?,
            discovered_at: DiscoveredAt::new(micros_to_datetime(self.discovered_ats.value(i))),
            original_at: OriginalAt::new(micros_to_datetime(self.original_ats.value(i))),
            zone,
            config_version,
            detected_text: self.detected_texts.value(i).to_string(),
        })
    }
}

#[instrument(skip(batches))]
fn batches_to_summaries(batches: &[RecordBatch]) -> Result<Vec<StoredImageSummary>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let cols = SummaryColumns::extract(batch)?;
        for i in 0..batch.num_rows() {
            results.push(cols.to_summary(i)?);
        }
    }
    Ok(results)
}

#[instrument(skip(batches))]
fn batches_to_stored_images(batches: &[RecordBatch]) -> Result<Vec<StoredImage>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let cols = SummaryColumns::extract(batch)?;
        let images = typed_column::<LargeBinaryArray>(batch, "image")?;
        let annotated_images = typed_column::<LargeBinaryArray>(batch, "annotated_image")?;

        for i in 0..batch.num_rows() {
            let summary = cols.to_summary(i)?;
            let image = image::load_from_memory(images.value(i))?;
            let annotated_image = image::load_from_memory(annotated_images.value(i))?;
            results.push(StoredImage {
                summary,
                image,
                annotated_image,
            });
        }
    }
    Ok(results)
}

#[instrument(skip(batch))]
fn typed_column<'a, T: Array + 'static>(
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a T, StoreError> {
    batch
        .column_by_name(name)
        .and_then(|col| col.as_any().downcast_ref::<T>())
        .ok_or_else(|| StoreError::ColumnTypeMismatch {
            column: name.to_string(),
        })
}

#[instrument(skip(img))]
fn encode_image_as_png(img: &DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

fn micros_to_datetime(micros: i64) -> DateTime<Utc> {
    Utc.timestamp_micros(micros)
        .single()
        .expect("valid timestamp from store")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};
    use shared::ConfigVersion;

    fn test_image() -> DynamicImage {
        test_image_with_color(255, 0, 0)
    }

    fn test_image_with_color(r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([r, g, b, 255])))
    }

    async fn open_temp_store(dir: &tempfile::TempDir) -> SkeetStore {
        SkeetStore::open(dir.path().to_str().expect("valid path"), vec![])
            .await
            .expect("open store")
    }

    #[tokio::test]
    async fn roundtrip_store_and_retrieve() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        assert_eq!(store.count().await.unwrap(), 0);

        let record = ImageRecord {
            image_id: ImageId::from_image(&test_image()),
            skeet_id: "at://did:plc:abc/app.bsky.feed.post/123"
                .parse()
                .expect("valid test AT URI"),
            image: test_image(),
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(Utc::now()),
            zone: Zone::TopRight,
            annotated_image: test_image(),
            config_version: ConfigVersion::from("test"),
            detected_text: String::new(),
        };

        store.add(&record).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1);

        let images = store.list_all().await.unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].summary.image_id, record.image_id);
        assert_eq!(images[0].summary.skeet_id, record.skeet_id);
        assert_eq!(images[0].image.width(), 2);
        assert_eq!(images[0].image.height(), 2);
        assert_eq!(images[0].summary.zone, Zone::TopRight);
    }

    #[tokio::test]
    async fn multiple_images_per_skeet() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/456"
            .parse()
            .expect("valid test AT URI");

        for i in 0..3 {
            let img = test_image_with_color(i * 80, 0, 0);
            let record = ImageRecord {
                image_id: ImageId::from_image(&img),
                skeet_id: skeet_id.clone(),
                image: img,
                discovered_at: DiscoveredAt::now(),
                original_at: OriginalAt::new(Utc::now()),
                zone: Zone::BottomLeft,
                annotated_image: test_image(),
                config_version: ConfigVersion::from("test"),
                detected_text: String::new(),
            };
            store.add(&record).await.unwrap();
        }

        assert_eq!(store.count().await.unwrap(), 3);

        let unique_skeets = store.unique_skeet_ids().await.unwrap();
        assert_eq!(unique_skeets.len(), 1);
        assert_eq!(unique_skeets[0], skeet_id);
    }

    #[tokio::test]
    async fn list_all_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        let record = ImageRecord {
            image_id: ImageId::from_image(&test_image()),
            skeet_id: "at://did:plc:abc/app.bsky.feed.post/summ"
                .parse()
                .expect("valid test AT URI"),
            image: test_image(),
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(Utc::now()),
            zone: Zone::BottomRight,
            annotated_image: test_image(),
            config_version: ConfigVersion::from("test"),
            detected_text: String::new(),
        };

        store.add(&record).await.unwrap();

        let summaries = store.list_all_summaries().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].image_id, record.image_id);
        assert_eq!(summaries[0].skeet_id, record.skeet_id);
        assert_eq!(summaries[0].zone, Zone::BottomRight);
    }

    #[tokio::test]
    async fn reopening_store_preserves_data() {
        let dir = tempfile::tempdir().unwrap();

        let record = ImageRecord {
            image_id: ImageId::from_image(&test_image()),
            skeet_id: "at://did:plc:abc/app.bsky.feed.post/789"
                .parse()
                .expect("valid test AT URI"),
            image: test_image(),
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(Utc::now()),
            zone: Zone::TopLeft,
            annotated_image: test_image(),
            config_version: ConfigVersion::from("test"),
            detected_text: String::new(),
        };

        {
            let store = open_temp_store(&dir).await;
            store.add(&record).await.unwrap();
        }

        let store = open_temp_store(&dir).await;
        assert_eq!(store.count().await.unwrap(), 1);
    }
}
