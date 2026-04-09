#![warn(clippy::all, clippy::nursery)]
mod args;
mod arrow_utils;
mod compact;
mod error;
pub mod health;
mod lancedb_utils;
mod schema;
mod scores;
mod stored;
mod summary;
mod types;

pub use args::StoreArgs;
pub use compact::CompactTarget;
pub use error::StoreError;
pub use shared::{ModelVersion, Score};
pub use stored::{StoredImage, StoredImageSummary};
pub use summary::SkeetStoreSummary;
pub use types::{DiscoveredAt, ImageId, ImageRecord, InvalidImageId, OriginalAt, SkeetId, Zone};

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use arrow_array::{
    Int64Array, LargeBinaryArray, RecordBatch, RecordBatchIterator, StringArray,
    TimestampMicrosecondArray,
};
use chrono::Utc;
use lancedb::index::Index;
use lancedb::query::QueryBase;
use tracing::{info, instrument};

use arrow_utils::{encode_image_as_png, min_max_timestamp, typed_column};
use lancedb_utils::execute_query;
use schema::{
    SCORE_TABLE_NAME, TABLE_NAME, VALIDATE_TABLE_NAME, images_score_v2_schema, images_v6_schema,
    validate_v1_schema,
};
use stored::{batches_to_stored_images, batches_to_summaries};

pub struct SkeetStore {
    pub(crate) images_table: lancedb::Table,
    pub(crate) scores_table: lancedb::Table,
    validate_table: lancedb::Table,
    pub(crate) writes_since_compact: AtomicU64,
    pub(crate) compact_every_n_writes: Option<u64>,
    pub(crate) scores_cache: RwLock<Option<scores::ScoresCache>>,
}

impl SkeetStore {
    #[instrument(skip(storage_options))]
    pub async fn open(
        uri: &str,
        storage_options: Vec<(String, String)>,
        compact_every_n_writes: Option<u64>,
    ) -> Result<Self, StoreError> {
        info!(uri, "opening store");
        let db = lancedb::connect(uri)
            .read_consistency_interval(Duration::ZERO)
            .storage_options(storage_options)
            .execute()
            .await?;

        let table_names = db.table_names().execute().await?;
        if !table_names.contains(&TABLE_NAME.to_string()) {
            db.create_empty_table(TABLE_NAME, images_v6_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&SCORE_TABLE_NAME.to_string()) {
            db.create_empty_table(SCORE_TABLE_NAME, images_score_v2_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&VALIDATE_TABLE_NAME.to_string()) {
            db.create_empty_table(VALIDATE_TABLE_NAME, validate_v1_schema())
                .execute()
                .await?;
        }

        let images_table = db.open_table(TABLE_NAME).execute().await?;
        let indices = images_table.list_indices().await?;
        if !indices.iter().any(|idx| idx.columns == vec!["image_id"]) {
            images_table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }
        if !indices
            .iter()
            .any(|idx| idx.columns == vec!["discovered_at"])
        {
            images_table
                .create_index(&["discovered_at"], Index::Auto)
                .execute()
                .await?;
        }

        let scores_table = db.open_table(SCORE_TABLE_NAME).execute().await?;
        let score_indices = scores_table.list_indices().await?;
        if !score_indices
            .iter()
            .any(|idx| idx.columns == vec!["image_id"])
        {
            scores_table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }
        if !score_indices
            .iter()
            .any(|idx| idx.columns == vec!["model_version"])
        {
            scores_table
                .create_index(&["model_version"], Index::Auto)
                .execute()
                .await?;
        }

        let validate_table = db.open_table(VALIDATE_TABLE_NAME).execute().await?;

        let images_stats = images_table.stats().await?;
        let scores_stats = scores_table.stats().await?;
        info!(?indices, ?images_stats, "images_table stats");
        info!(?score_indices, ?scores_stats, "scores_table stats");

        for idx in &indices {
            let stats = images_table.index_stats(&idx.name).await?;
            info!(index_name = %idx.name, ?stats, "images_table index stats");
        }
        for idx in &score_indices {
            let stats = scores_table.index_stats(&idx.name).await?;
            info!(index_name = %idx.name, ?stats, "scores_table index stats");
        }

        info!(uri, ?compact_every_n_writes, "store opened");
        Ok(Self {
            images_table,
            scores_table,
            validate_table,
            writes_since_compact: AtomicU64::new(0),
            compact_every_n_writes,
            scores_cache: RwLock::new(None),
        })
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

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.images_table.add(batches).execute().await?;
        self.compact_if_needed().await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn list_all(&self) -> Result<Vec<StoredImage>, StoreError> {
        let batches = execute_query(&self.images_table.query(), "list_all").await?;
        batches_to_stored_images(&batches)
    }

    #[instrument(skip(self))]
    pub async fn list_all_by_most_recent(&self) -> Result<Vec<StoredImage>, StoreError> {
        let mut images = self.list_all().await?;
        images.sort_by(|a, b| b.summary.discovered_at.cmp(&a.summary.discovered_at));
        Ok(images)
    }

    #[instrument(skip(self))]
    pub async fn list_all_summaries(&self) -> Result<Vec<StoredImageSummary>, StoreError> {
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "skeet_id",
                "discovered_at",
                "original_at",
                "archetype",
                "config_version",
                "detected_text",
            ]));
        let batches = execute_query(&query, "list_all_summaries").await?;
        batches_to_summaries(&batches)
    }

    #[instrument(skip(self))]
    pub async fn get_by_id(&self, image_id: &ImageId) -> Result<Option<StoredImage>, StoreError> {
        let query = self
            .images_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1);
        let batches = execute_query(&query, "get_by_id").await?;
        Ok(batches_to_stored_images(&batches)?.into_iter().next())
    }

    #[instrument(skip(self))]
    pub async fn exists(&self, image_id: &ImageId) -> Result<bool, StoreError> {
        let query = self
            .images_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .select(lancedb::query::Select::columns(&["image_id"]))
            .limit(1);
        let batches = execute_query(&query, "exists").await?;
        Ok(batches.iter().any(|b| b.num_rows() > 0))
    }

    #[instrument(skip(self))]
    pub async fn delete_by_id(&self, image_id: &ImageId) -> Result<(), StoreError> {
        self.images_table
            .delete(&format!("image_id = '{image_id}'"))
            .await?;
        self.compact_if_needed().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn count(&self) -> Result<usize, StoreError> {
        Ok(self.images_table.count_rows(None).await?)
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

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.validate_table.add(batches).execute().await?;

        let query = self
            .validate_table
            .query()
            .only_if(format!("random_number = {random_number}"));
        let result_batches = execute_query(&query, "validate").await?;

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
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&["skeet_id"]));
        let batches = execute_query(&query, "unique_skeet_ids").await?;

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

    #[instrument(skip(self))]
    pub async fn list_all_image_ids_by_most_recent(&self) -> Result<Vec<ImageId>, StoreError> {
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "discovered_at",
            ]));
        let batches = execute_query(&query, "list_all_image_ids_by_most_recent").await?;

        let mut id_times = Vec::new();
        for batch in &batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let discovered_ats =
                typed_column::<TimestampMicrosecondArray>(batch, "discovered_at")?;
            for i in 0..batch.num_rows() {
                id_times.push((image_ids.value(i).parse::<ImageId>()?, discovered_ats.value(i)));
            }
        }
        id_times.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(id_times.into_iter().map(|(id, _)| id).collect())
    }

    #[instrument(skip(self))]
    pub async fn summarise(&self) -> Result<SkeetStoreSummary, StoreError> {
        let image_count = self.images_table.count_rows(None).await?;
        let score_count = self.scores_table.count_rows(None).await?;

        let timestamps_query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "discovered_at",
                "original_at",
            ]));
        let batches = execute_query(&timestamps_query, "summarise:timestamps").await?;

        let discovered_at_range = min_max_timestamp(&batches, "discovered_at")?
            .map(|(min, max)| (DiscoveredAt::new(min), DiscoveredAt::new(max)));
        let original_at_range = min_max_timestamp(&batches, "original_at")?
            .map(|(min, max)| (OriginalAt::new(min), OriginalAt::new(max)));

        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]));
        let scored_batches = execute_query(&scored_query, "summarise:scored_ids").await?;

        let mut scored_ids = std::collections::HashSet::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                scored_ids.insert(image_ids.value(i).to_string());
            }
        }

        Ok(SkeetStoreSummary {
            image_count,
            score_count,
            scored_image_count: scored_ids.len(),
            discovered_at_range,
            original_at_range,
        })
    }
}

#[cfg(test)]
mod store_tests;
