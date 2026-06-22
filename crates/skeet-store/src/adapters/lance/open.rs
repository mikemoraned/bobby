use std::sync::Arc;
use std::time::Duration;

use enum_map::EnumMap;
use lance::dataset::ReadParams;
use lance_io::object_store::ObjectStoreParams;
use lancedb::index::Index;
use tokio::sync::RwLock;
use tracing::{info, instrument};

use super::schema::TableName;
use crate::adapters::object_store::R2MetricsWrapper;
use crate::error::StoreError;
use crate::{SkeetStore, VersionedCache};

impl SkeetStore {
    #[instrument(skip(storage_options))]
    pub async fn open(
        uri: &str,
        storage_options: Vec<(String, String)>,
        cli_name: &str,
    ) -> Result<Self, StoreError> {
        info!(uri, cli_name, "opening store");
        let db = lancedb::connect(uri)
            .read_consistency_interval(Duration::ZERO)
            .storage_options(storage_options)
            .execute()
            .await?;

        let store_wrapper = Arc::new(R2MetricsWrapper::new(
            cli_name,
            opentelemetry::global::meter("r2"),
        ));
        let read_params = ReadParams {
            store_options: Some(ObjectStoreParams {
                object_store_wrapper: Some(store_wrapper.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let existing = db.table_names().execute().await?;
        let specs: EnumMap<TableName, _> = EnumMap::from_fn(TableName::spec);

        // Create-if-missing, open, and index-if-missing every table from its
        // declarative spec — one pass, so "add a table" is a single `spec` arm.
        let mut tables: EnumMap<TableName, Option<lancedb::Table>> = EnumMap::default();
        for (name, spec) in &specs {
            if !existing.iter().any(|n| n == name.as_str()) {
                db.create_empty_table(name.as_str(), (spec.schema)())
                    .execute()
                    .await?;
            }
            let table = db
                .open_table(name.as_str())
                .lance_read_params(read_params.clone())
                .execute()
                .await?;
            ensure_indices(&table, spec.indexed_columns).await?;
            log_table_stats(name, &table).await?;
            tables[name] = Some(table);
        }

        info!(uri, "store opened");
        // Every variant is populated above (the `specs` from_fn yields all of
        // them), so each entry is `Some`.
        #[allow(clippy::expect_used)]
        let tables = tables.map(|_, table| table.expect("every table opened above"));
        Ok(Self {
            tables,
            scores_cache: RwLock::new(VersionedCache::new()),
            store_wrapper,
        })
    }
}

/// Create each missing BTree index on `table`, leaving existing ones untouched.
async fn ensure_indices(table: &lancedb::Table, columns: &[&str]) -> Result<(), StoreError> {
    let existing = table.list_indices().await?;
    for &column in columns {
        if !existing.iter().any(|idx| idx.columns == [column]) {
            table.create_index(&[column], Index::Auto).execute().await?;
        }
    }
    Ok(())
}

async fn log_table_stats(name: TableName, table: &lancedb::Table) -> Result<(), StoreError> {
    let stats = table.stats().await?;
    let indices = table.list_indices().await?;
    info!(table = %name, ?indices, ?stats, "table stats");
    for idx in &indices {
        let idx_stats = table.index_stats(&idx.name).await?;
        info!(table = %name, index_name = %idx.name, ?idx_stats, "table index stats");
    }
    Ok(())
}
