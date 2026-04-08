use std::sync::atomic::Ordering;

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

impl super::SkeetStore {
    #[instrument(skip(self))]
    pub async fn compact(&self) -> Result<(), StoreError> {
        self.compact_table(CompactTarget::All).await
    }

    #[instrument(skip(self))]
    pub async fn compact_table(&self, target: CompactTarget) -> Result<(), StoreError> {
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
        let images_compact_options = CompactionOptions {
            num_threads: Some(1),
            target_rows_per_fragment: 500,
            batch_size: Some(64),
            ..CompactionOptions::default()
        };
        // scores_table rows are small (image_id + f32 + model_version), so the
        // default target_rows_per_fragment (1M) is fine — memory is not a concern.
        let scores_compact_options = CompactionOptions {
            num_threads: Some(1),
            ..CompactionOptions::default()
        };

        if target.includes_images() {
            let before = self.images_table.stats().await?;
            info!(?before, "compacting images_table");
            self.images_table
                .optimize(OptimizeAction::Compact {
                    options: images_compact_options,
                    remap_options: None,
                })
                .await?;
            info!("images_table compaction done, rebuilding indices");
            self.images_table
                .optimize(OptimizeAction::Index(Default::default()))
                .await?;
            let after = self.images_table.stats().await?;
            info!(?after, "images_table optimization complete");
        }

        if target.includes_scores() {
            let before = self.scores_table.stats().await?;
            info!(?before, "compacting scores_table");
            self.scores_table
                .optimize(OptimizeAction::Compact {
                    options: scores_compact_options,
                    remap_options: None,
                })
                .await?;
            info!("scores_table compaction done, rebuilding indices");
            self.scores_table
                .optimize(OptimizeAction::Index(Default::default()))
                .await?;
            let after = self.scores_table.stats().await?;
            info!(?after, "scores_table optimization complete");
        }

        self.writes_since_compact.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub async fn storage_health(&self) -> Result<health::StoreHealth, StoreError> {
        let mut tables = Vec::new();

        for (name, table) in [
            ("images_table", &self.images_table),
            ("scores_table", &self.scores_table),
        ] {
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

    pub(crate) async fn compact_if_needed(&self) -> Result<(), StoreError> {
        if let Some(threshold) = self.compact_every_n_writes {
            let count = self.writes_since_compact.fetch_add(1, Ordering::Relaxed) + 1;
            if count >= threshold {
                self.compact().await?;
            }
        }
        Ok(())
    }
}
