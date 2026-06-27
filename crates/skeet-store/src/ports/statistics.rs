use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{PruneStats, StoreError};

/// Prune statistics: per-interval tallies of what the pruner saw on the
/// firehose, plus aggregate queries over the recorded intervals.
#[async_trait]
pub trait Statistics: Send + Sync {
    /// Append one interval's prune statistics.
    async fn record(&self, stats: &PruneStats) -> Result<(), StoreError>;

    /// The latest `interval_end` over all recorded intervals, or `None` when no
    /// statistics have been recorded yet — the resume point for backfilling.
    async fn latest_interval_end(&self) -> Result<Option<DateTime<Utc>>, StoreError>;

    /// Combine every recorded interval whose `interval_start` falls in the
    /// half-open window `[start, end)` into a single [`PruneStats`]: the counts
    /// sum and the bounds widen to the covered span (earliest start, latest end)
    /// of the contributing records. An empty window returns zero counts bounded
    /// by the queried `[start, end)`.
    async fn interval_counts(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<PruneStats, StoreError>;
}
