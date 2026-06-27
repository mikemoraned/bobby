use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{PruneStats, StoreError};

/// Prune statistics: per-interval tallies of what the pruner saw on the
/// firehose, plus aggregate queries over the recorded intervals.
#[async_trait]
pub trait Statistics: Send + Sync {
    /// Append one interval's prune statistics.
    async fn record(&self, stats: &PruneStats) -> Result<(), StoreError>;

    /// Sum the counts of every recorded interval whose `interval_start` falls in
    /// the half-open window `[start, end)`, returned as a single [`PruneStats`]
    /// spanning that window (its `interval_start`/`interval_end` are `start`/`end`).
    async fn interval_counts(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<PruneStats, StoreError>;
}
