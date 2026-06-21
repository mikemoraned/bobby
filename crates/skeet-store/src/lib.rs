#![warn(clippy::all, clippy::nursery)]
mod appraisals;
mod args;
mod arrow_utils;
mod error;
pub mod health;
mod images;
mod lancedb_utils;
mod open;
mod optimise;
mod r2_metrics;
mod schema;
mod scored_view;
mod scores;
pub mod store_metrics;
mod stored;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_utils;
mod types;
mod version;
pub mod versioned_cache;

pub use appraisals::{Appraisal, AppraisalSource, Appraisals};
pub use args::StoreArgs;
pub use error::StoreError;
pub use images::Images;
pub use schema::{
    IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME, TABLE_NAME,
    VALIDATE_TABLE_NAME,
};
pub use scored_view::ScoredView;
pub use scores::Scores;
pub use shared::{Appraiser, Band, ImageId, ModelVersion, Score};
pub use store_metrics::StoreMetrics;
pub use stored::{StoredImage, StoredImageSummary, StoredOriginal};
pub use types::{DiscoveredAt, ImageRecord, OriginalAt, SkeetId, Zone};
pub use version::{TableVersions, Version};
pub use versioned_cache::VersionedCache;

use std::sync::Arc;

use lance_io::object_store::WrappingObjectStore;
use tokio::sync::RwLock;

use arrow_array::{Int64Array, RecordBatch, TimestampMicrosecondArray};
use chrono::Utc;
use lancedb::query::QueryBase;
use tracing::instrument;

use arrow_utils::typed_column;
use lance::dataset::{WriteMode, WriteParams};
use lance_io::object_store::ObjectStoreParams;
use lancedb::table::WriteOptions;
use lancedb_utils::execute_query;
use schema::validate_v1_schema;

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
    pub(crate) scores_cache: RwLock<VersionedCache<Version, scored_view::ScoresMap>>,
    pub(crate) store_wrapper: Arc<dyn WrappingObjectStore>,
}

impl SkeetStore {
    /// Build `WriteOptions` that include the R2 metrics wrapper, if configured.
    pub(crate) fn write_options(&self) -> WriteOptions {
        WriteOptions {
            lance_write_params: Some(WriteParams {
                mode: WriteMode::Append,
                store_params: Some(ObjectStoreParams {
                    object_store_wrapper: Some(self.store_wrapper.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }
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
}

#[cfg(test)]
mod store_tests;
