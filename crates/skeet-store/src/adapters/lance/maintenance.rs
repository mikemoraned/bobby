use lancedb::table::{CompactionOptions, OptimizeAction};
use tracing::{info, instrument};

use super::schema::TABLE_NAME;
use crate::StoreError;
use crate::health;

// images_table stores ~2MB PNG blobs per row. Lance's compaction planner
// treats any fragment with physical_rows < target_rows_per_fragment as a
// candidate — see the candidacy check in plan_compaction():
//   https://github.com/lancedb/lance/blob/v0.20.0/rust/lance/src/dataset/optimize.rs#L693
// CompactionOptions fields documented at:
//   https://docs.rs/lancedb/0.26.2/lancedb/table/struct.CompactionOptions.html
//
// With the default of 1M, all fragments are candidates and lance tries to
// merge them into one massive fragment, requiring all image data in memory.
//
// Setting target_rows_per_fragment=500 means:
//  - fragments with >500 rows are NOT candidates (left alone)
//  - single-row fragments from add() ARE candidates, merged into ~500-row
//    groups (~1GB), well within k8s memory limits
//  - num_threads=1 ensures only one compaction task runs at a time
//  - batch_size=64 limits the scanner read batch during compaction
fn compact_options_for(name: &str) -> CompactionOptions {
    if name == TABLE_NAME {
        CompactionOptions {
            num_threads: Some(1),
            target_rows_per_fragment: 500,
            batch_size: Some(64),
            ..CompactionOptions::default()
        }
    } else {
        // Other tables (scores, validate, appraisals) hold small rows — the
        // default target_rows_per_fragment (1M) is fine, memory is not a concern.
        CompactionOptions {
            num_threads: Some(1),
            ..CompactionOptions::default()
        }
    }
}

impl super::SkeetStore {
    /// All `(name, table, compact_options)` triples drawn from the
    /// `SkeetStore::tables` registry. Single source of truth for which tables
    /// a maintenance op walks.
    fn maintenance_tables(&self) -> Vec<(&'static str, &lancedb::Table, CompactionOptions)> {
        self.tables
            .iter()
            .map(|(name, table)| (*name, table, compact_options_for(name)))
            .collect()
    }

    #[instrument(skip(self))]
    pub async fn optimise(&self) -> Result<(), StoreError> {
        for (name, table, options) in self.maintenance_tables() {
            compact_and_reindex(name, table, options).await?;
        }
        Ok(())
    }

    /// Prune `_versions/` manifests older than 1h. Without this, manifests
    /// accumulate forever and every Strong-mode read pays a growing R2 LIST
    /// cost. 1h is paired with the 10-min optimise cron cadence — long enough
    /// to never race an in-flight read, short enough to keep the active
    /// manifest count to a single R2 LIST page.
    #[instrument(skip(self))]
    pub async fn prune_old_versions(&self) -> Result<(), StoreError> {
        let older_than = chrono::Duration::hours(1);
        for (name, table, _) in self.maintenance_tables() {
            prune_versions(name, table, older_than).await?;
        }
        Ok(())
    }

    pub async fn storage_health(&self) -> Result<health::StoreHealth, StoreError> {
        let mut tables = Vec::new();

        for (name, table, _) in self.maintenance_tables() {
            let stats = table.stats().await?;
            let indices = table.list_indices().await?;
            let mut index_health = Vec::new();
            for idx in &indices {
                let idx_stats = table.index_stats(&idx.name).await?;
                index_health.push(health::IndexHealth {
                    name: idx.name.clone(),
                    stats: idx_stats,
                });
            }
            tables.push(health::TableHealth {
                name: name.to_string(),
                stats,
                index_health,
            });
        }

        Ok(health::StoreHealth { tables })
    }
}

async fn compact_and_reindex(
    name: &str,
    table: &lancedb::Table,
    options: CompactionOptions,
) -> Result<(), StoreError> {
    let before = table.stats().await?;
    info!(table = name, ?before, "compacting");
    table
        .optimize(OptimizeAction::Compact {
            options,
            remap_options: None,
        })
        .await?;
    info!(table = name, "compaction done, rebuilding indices");
    table
        .optimize(OptimizeAction::Index(Default::default()))
        .await?;
    let after = table.stats().await?;
    info!(table = name, ?after, "optimization complete");
    Ok(())
}

async fn prune_versions(
    name: &str,
    table: &lancedb::Table,
    older_than: chrono::Duration,
) -> Result<(), StoreError> {
    info!(table = name, "pruning old versions");
    let stats = table
        .optimize(OptimizeAction::Prune {
            older_than: Some(older_than),
            delete_unverified: None,
            error_if_tagged_old_versions: None,
        })
        .await?;
    info!(table = name, ?stats.prune, "prune complete");
    Ok(())
}
