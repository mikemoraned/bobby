use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{ModelVersion, PruneStats, StoreError};

/// Store statistics: per-interval tallies of what the pruner saw on the firehose
/// and aggregate queries over the recorded intervals, plus the aggregate row
/// counts over the images and scores tables.
#[async_trait]
pub trait Statistics: Send + Sync {
    /// Append one interval's prune statistics.
    async fn record_prune_stats(&self, stats: &PruneStats) -> Result<(), StoreError>;

    /// The latest `interval_end` over all recorded prune-stats intervals, or
    /// `None` when none have been recorded yet — i.e. how far forward the recorded
    /// statistics reach.
    async fn latest_prune_stats_interval_end(&self)
    -> Result<Option<DateTime<Utc>>, StoreError>;

    /// Combine every recorded interval whose `interval_start` falls in the
    /// half-open window `[start, end)` into a single [`PruneStats`]: the counts
    /// sum and the bounds widen to the covered span (earliest start, latest end)
    /// of the contributing records. An empty window returns zero counts bounded
    /// by the queried `[start, end)`.
    async fn prune_stats_for_interval(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<PruneStats, StoreError>;

    /// Total number of stored images.
    async fn count_images(&self) -> Result<usize, StoreError>;

    /// Count images whose `discovered_at` falls in the half-open window
    /// `[start, end)`.
    async fn count_images_in_interval(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<u64, StoreError>;

    /// Count the distinct images that have a score from a *known* model version —
    /// the "images examined" total, i.e. images that made it through refine
    /// scoring. Each image is counted once; scores from unregistered model
    /// versions are excluded, mirroring the feed read path (see
    /// `docs/versioning.md`).
    async fn count_scored_images(
        &self,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<usize, StoreError>;

    /// Scan all scores and return a count per distinct `model_version`.
    async fn count_scores_by_model_version(&self) -> Result<HashMap<String, usize>, StoreError>;
}
