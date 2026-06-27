use async_trait::async_trait;

use crate::pipeline::ContentCounts;

/// A sink for the per-message content tallies that flow into the
/// content-statistics stage.
#[async_trait]
pub trait ContentCountsRecorder {
    async fn record_counts(&mut self, counts: &ContentCounts);
}
