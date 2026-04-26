#![warn(clippy::all, clippy::nursery)]
mod appraisals;
mod args;
mod arrow_utils;
mod compact;
mod error;
pub mod health;
mod lancedb_utils;
mod open;
mod paging;
mod r2_metrics;
mod schema;
pub mod store_metrics;
mod scores;
mod stored;
mod summary;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_utils;
pub mod tempo;
pub mod trace_analysis;
mod types;
mod version;

pub use appraisals::Appraisal;
pub use args::StoreArgs;
pub use compact::CompactTarget;
pub use error::StoreError;
pub use schema::{
    IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME, TABLE_NAME,
    VALIDATE_TABLE_NAME,
};
pub use shared::{Appraiser, Band, ModelVersion, Score};
pub use stored::{StoredImage, StoredImageSummary, StoredOriginal};
pub use store_metrics::StoreMetrics;
pub use summary::SkeetStoreSummary;
pub use types::{DiscoveredAt, ImageId, ImageRecord, InvalidImageId, OriginalAt, SkeetId, Zone};
pub use version::Version;

use std::sync::Arc;

use lance_io::object_store::WrappingObjectStore;
use tokio::sync::RwLock;

use arrow_array::{
    Int64Array, LargeBinaryArray, RecordBatch, StringArray,
    TimestampMicrosecondArray,
};
use chrono::Utc;
use lancedb::query::QueryBase;
use tracing::instrument;

use arrow_utils::{encode_image_as_png, typed_column};
use lance::dataset::{WriteMode, WriteParams};
use lance_io::object_store::ObjectStoreParams;
use lancedb::table::WriteOptions;
use lancedb_utils::execute_query;
use schema::{images_v6_schema, validate_v1_schema};
use stored::{batches_to_original_images, batches_to_stored_images, batches_to_summaries};

pub struct SkeetStore {
    pub(crate) images_table: lancedb::Table,
    pub(crate) scores_table: lancedb::Table,
    validate_table: lancedb::Table,
    pub(crate) skeet_appraisal_table: lancedb::Table,
    pub(crate) image_appraisal_table: lancedb::Table,
    /// All tables, paired with their canonical name. Source of truth for
    /// per-table iteration (fragment counts, version snapshots). Populated in
    /// `SkeetStore::open` so adding or removing a table is a single edit.
    pub(crate) tables: Vec<(&'static str, lancedb::Table)>,
    pub(crate) scores_cache: RwLock<Option<scores::ScoresCache>>,
    pub(crate) store_wrapper: Option<Arc<dyn WrappingObjectStore>>,
}

impl SkeetStore {
    /// Return the current version counter for each table. Cheap: reads only the cached manifest.
    pub async fn table_versions(&self) -> Result<Vec<(&'static str, u64)>, StoreError> {
        let mut versions = Vec::with_capacity(self.tables.len());
        for (name, table) in &self.tables {
            let v = table.version().await?;
            versions.push((*name, v));
        }
        Ok(versions)
    }

    /// Return the fragment count for each table.
    /// Cheap: reads only the cached manifest, no per-fragment or per-column I/O.
    pub async fn fragment_counts(&self) -> Result<Vec<(&'static str, u64)>, StoreError> {
        let mut counts = Vec::with_capacity(self.tables.len());
        for (name, table) in &self.tables {
            let native = table
                .as_native()
                .ok_or_else(|| StoreError::CannotGetFragmentCount {
                    table: (*name).to_string(),
                    reason: "table is not a native LanceDB table".to_string(),
                })?;
            let count = native.count_fragments().await?;
            counts.push((*name, count as u64));
        }
        Ok(counts)
    }

    /// Build `WriteOptions` that include the R2 metrics wrapper, if configured.
    pub(crate) fn write_options(&self) -> WriteOptions {
        WriteOptions {
            lance_write_params: self.store_wrapper.as_ref().map(|wrapper| WriteParams {
                mode: WriteMode::Append,
                store_params: Some(ObjectStoreParams {
                    object_store_wrapper: Some(wrapper.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }
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

        self.images_table
            .add(vec![batch])
            .write_options(self.write_options())
            .execute()
            .await?;

        Ok(())
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

    #[instrument(skip(self, image_ids), fields(count = image_ids.len()))]
    pub async fn get_by_ids(&self, image_ids: &[ImageId]) -> Result<Vec<StoredImage>, StoreError> {
        if image_ids.is_empty() {
            return Ok(vec![]);
        }
        let query = self
            .images_table
            .query()
            .only_if(id_in_list_filter(image_ids));
        let batches = execute_query(&query, "get_by_ids").await?;
        batches_to_stored_images(&batches)
    }

    #[instrument(skip(self, image_ids), fields(count = image_ids.len()))]
    pub async fn get_originals_by_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<Vec<StoredOriginal>, StoreError> {
        if image_ids.is_empty() {
            return Ok(vec![]);
        }
        let query = self
            .images_table
            .query()
            .only_if(id_in_list_filter(image_ids))
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "skeet_id",
                "discovered_at",
                "original_at",
                "archetype",
                "config_version",
                "detected_text",
                "image",
            ]));
        let batches = execute_query(&query, "get_originals_by_ids").await?;
        batches_to_original_images(&batches)
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

        self.validate_table
            .add(vec![batch])
            .write_options(self.write_options())
            .execute()
            .await?;

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

}

fn id_in_list_filter(image_ids: &[ImageId]) -> String {
    let id_list = image_ids
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("image_id IN ({id_list})")
}

#[cfg(test)]
mod store_tests;
