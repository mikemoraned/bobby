//! The LanceDB/R2 adapter: the concrete implementation of store ports.
//!
//! Everything that knows about LanceDB tables, Arrow record batches, and query
//! execution lives here and stays private to this module — `ports` and `model`
//! cannot name a `lancedb::Table` or an Arrow array.

mod appraisals;
mod arrow;
mod decode;
mod images;
mod maintenance;
mod open;
mod query;
mod schema;
mod scored_view;
mod scores;
mod versions;

pub use schema::TableName;

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, TimestampMicrosecondArray};
use chrono::Utc;
use enum_map::EnumMap;
use lance::dataset::{WriteMode, WriteParams};
use lance_io::object_store::{ObjectStoreParams, WrappingObjectStore};
use lancedb::query::QueryBase;
use lancedb::table::WriteOptions;
use tokio::sync::RwLock;
use tracing::instrument;

use self::arrow::typed_column;
use self::query::execute_query;
use self::schema::validate_v1_schema;
use crate::{StoreError, Version, VersionedCache, model};

pub struct SkeetStore {
    /// Every table handle, keyed by [`TableName`]. The single source of truth for
    /// both direct access (`self.table(TableName::Images)`) and whole-store
    /// iteration (fragment counts, version snapshots, maintenance); a total map,
    /// so every variant is present by construction (built in `SkeetStore::open`).
    pub(in crate::adapters::lance) tables: EnumMap<TableName, lancedb::Table>,
    pub(in crate::adapters::lance) scores_cache: RwLock<VersionedCache<Version, model::ScoresMap>>,
    pub(in crate::adapters::lance) store_wrapper: Arc<dyn WrappingObjectStore>,
}

impl SkeetStore {
    /// The handle for one table. Infallible — the map is total over [`TableName`].
    pub(in crate::adapters::lance) fn table(&self, name: TableName) -> &lancedb::Table {
        &self.tables[name]
    }

    /// Build `WriteOptions` that include the R2 metrics wrapper, if configured.
    pub(in crate::adapters::lance) fn write_options(&self) -> WriteOptions {
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

        self.table(TableName::Validate)
            .add(vec![batch])
            .write_options(self.write_options())
            .execute()
            .await?;

        let query = self
            .table(TableName::Validate)
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
