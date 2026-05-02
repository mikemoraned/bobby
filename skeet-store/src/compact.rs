use lancedb::table::{CompactionOptions, OptimizeAction};
use tracing::{info, instrument};

use crate::health;
use crate::StoreError;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CompactTarget {
    All,
    Images,
    Scores,
}

impl CompactTarget {
    const fn includes_images(self) -> bool {
        matches!(self, Self::All | Self::Images)
    }

    const fn includes_scores(self) -> bool {
        matches!(self, Self::All | Self::Scores)
    }
}

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
fn images_compact_options() -> CompactionOptions {
    CompactionOptions {
        num_threads: Some(1),
        target_rows_per_fragment: 500,
        batch_size: Some(64),
        ..CompactionOptions::default()
    }
}

// scores_table rows are small (image_id + f32 + model_version), so the
// default target_rows_per_fragment (1M) is fine — memory is not a concern.
fn scores_compact_options() -> CompactionOptions {
    CompactionOptions {
        num_threads: Some(1),
        ..CompactionOptions::default()
    }
}

impl super::SkeetStore {
    /// All `(name, table, compact_options)` triples selected by `target`.
    /// Single source of truth for which tables a maintenance op walks, and
    /// the compaction options to use for each.
    fn selected_tables(
        &self,
        target: CompactTarget,
    ) -> Vec<(&'static str, &lancedb::Table, CompactionOptions)> {
        let mut out = Vec::new();
        if target.includes_images() {
            out.push(("images_table", &self.images_table, images_compact_options()));
        }
        if target.includes_scores() {
            out.push(("scores_table", &self.scores_table, scores_compact_options()));
        }
        out
    }

    #[instrument(skip(self))]
    pub async fn compact(&self) -> Result<(), StoreError> {
        self.compact_table(CompactTarget::All).await
    }

    #[instrument(skip(self))]
    pub async fn compact_table(&self, target: CompactTarget) -> Result<(), StoreError> {
        for (name, table, options) in self.selected_tables(target) {
            compact_and_reindex(name, table, options).await?;
        }
        Ok(())
    }

    /// Prune `_versions/` manifests older than 1h. Without this, manifests
    /// accumulate forever and every Strong-mode read pays a growing R2 LIST
    /// cost. 1h is paired with the 10-min compact cron cadence — long enough
    /// to never race an in-flight read, short enough to keep the active
    /// manifest count to a single R2 LIST page.
    #[instrument(skip(self))]
    pub async fn prune_old_versions(&self, target: CompactTarget) -> Result<(), StoreError> {
        let older_than = chrono::Duration::hours(1);
        for (name, table, _) in self.selected_tables(target) {
            prune_versions(name, table, older_than).await?;
        }
        Ok(())
    }

    pub async fn storage_health(&self) -> Result<health::StoreHealth, StoreError> {
        let mut tables = Vec::new();

        for (name, table, _) in self.selected_tables(CompactTarget::All) {
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
