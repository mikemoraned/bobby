mod error;
mod schema;
mod types;

pub use error::StoreError;
pub use types::{DiscoveredAt, ImageId, ImageRecord, OriginalAt, SkeetId};

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    LargeBinaryArray, RecordBatch, RecordBatchIterator, StringArray, TimestampMicrosecondArray,
};
use chrono::{DateTime, TimeZone, Utc};
use futures::TryStreamExt;
use image::DynamicImage;
use lancedb::query::{ExecutableQuery, QueryBase};

use schema::{TABLE_NAME, images_v1_schema};

pub struct SkeetStore {
    db: lancedb::Connection,
}

impl SkeetStore {
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let path_str = path
            .to_str()
            .ok_or_else(|| StoreError::InvalidPath(path.to_path_buf()))?;
        let db = lancedb::connect(path_str).execute().await?;

        let table_names = db.table_names().execute().await?;
        if !table_names.contains(&TABLE_NAME.to_string()) {
            db.create_empty_table(TABLE_NAME, images_v1_schema())
                .execute()
                .await?;
        }

        Ok(Self { db })
    }

    pub async fn add(&self, record: &ImageRecord) -> Result<(), StoreError> {
        let schema = images_v1_schema();

        let image_bytes = encode_image_as_png(&record.image)?;

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![record.image_id.as_str()])),
                Arc::new(StringArray::from(vec![record.skeet_id.as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&image_bytes])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.discovered_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.original_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
            ],
        )?;

        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table.add(batches).execute().await?;

        Ok(())
    }

    pub async fn list_all(&self) -> Result<Vec<StoredImage>, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        let batches: Vec<RecordBatch> = table.query().execute().await?.try_collect().await?;

        let mut results = Vec::new();
        for batch in &batches {
            let image_ids = batch
                .column_by_name("image_id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let skeet_ids = batch
                .column_by_name("skeet_id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            let image_data = batch
                .column_by_name("image_data")
                .unwrap()
                .as_any()
                .downcast_ref::<LargeBinaryArray>()
                .unwrap();
            let discovered_ats = batch
                .column_by_name("discovered_at")
                .unwrap()
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            let original_ats = batch
                .column_by_name("original_at")
                .unwrap()
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();

            for i in 0..batch.num_rows() {
                let image = image::load_from_memory(image_data.value(i))
                    .map_err(StoreError::ImageEncoding)?;
                results.push(StoredImage {
                    image_id: image_ids.value(i).parse().expect("valid UUID in store"),
                    skeet_id: SkeetId::new(skeet_ids.value(i)),
                    image,
                    discovered_at: micros_to_datetime(discovered_ats.value(i)),
                    original_at: micros_to_datetime(original_ats.value(i)),
                });
            }
        }

        Ok(results)
    }

    pub async fn count(&self) -> Result<usize, StoreError> {
        let table = self.db.open_table(TABLE_NAME).execute().await?;
        Ok(table.count_rows(None).await?)
    }

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
            let skeet_ids = batch
                .column_by_name("skeet_id")
                .unwrap()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            for i in 0..batch.num_rows() {
                let id = skeet_ids.value(i).to_string();
                if seen.insert(id.clone()) {
                    ids.push(SkeetId::new(id));
                }
            }
        }

        Ok(ids)
    }
}

pub struct StoredImage {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub image: DynamicImage,
    pub discovered_at: DateTime<Utc>,
    pub original_at: DateTime<Utc>,
}

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

    fn test_image() -> DynamicImage {
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255])))
    }

    #[tokio::test]
    async fn roundtrip_store_and_retrieve() {
        let dir = tempfile::tempdir().unwrap();
        let store = SkeetStore::open(dir.path()).await.unwrap();

        assert_eq!(store.count().await.unwrap(), 0);

        let record = ImageRecord {
            image_id: ImageId::new(),
            skeet_id: SkeetId::new("at://did:plc:abc/app.bsky.feed.post/123"),
            image: test_image(),
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(Utc::now()),
        };

        store.add(&record).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1);

        let images = store.list_all().await.unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].image_id, record.image_id);
        assert_eq!(images[0].skeet_id, record.skeet_id);
        assert_eq!(images[0].image.width(), 2);
        assert_eq!(images[0].image.height(), 2);
    }

    #[tokio::test]
    async fn multiple_images_per_skeet() {
        let dir = tempfile::tempdir().unwrap();
        let store = SkeetStore::open(dir.path()).await.unwrap();

        let skeet_id = SkeetId::new("at://did:plc:abc/app.bsky.feed.post/456");

        for _ in 0..3 {
            let record = ImageRecord {
                image_id: ImageId::new(),
                skeet_id: skeet_id.clone(),
                image: test_image(),
                discovered_at: DiscoveredAt::now(),
                original_at: OriginalAt::new(Utc::now()),
            };
            store.add(&record).await.unwrap();
        }

        assert_eq!(store.count().await.unwrap(), 3);

        let unique_skeets = store.unique_skeet_ids().await.unwrap();
        assert_eq!(unique_skeets.len(), 1);
        assert_eq!(unique_skeets[0], skeet_id);
    }

    #[tokio::test]
    async fn reopening_store_preserves_data() {
        let dir = tempfile::tempdir().unwrap();

        let record = ImageRecord {
            image_id: ImageId::new(),
            skeet_id: SkeetId::new("at://did:plc:abc/app.bsky.feed.post/789"),
            image: test_image(),
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(Utc::now()),
        };

        {
            let store = SkeetStore::open(dir.path()).await.unwrap();
            store.add(&record).await.unwrap();
        }

        let store = SkeetStore::open(dir.path()).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1);
    }
}
