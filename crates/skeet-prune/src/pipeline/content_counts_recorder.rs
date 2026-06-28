use async_trait::async_trait;

use crate::pipeline::ContentCounts;

/// A sink for the per-message content tallies that flow into the
/// content-statistics stage.
#[async_trait]
pub trait ContentCountsRecorder {
    async fn record_counts(&mut self, counts: &ContentCounts);

    /// Persist any state buffered short of its flush cadence. Called once when
    /// the stage shuts down so a partial batch isn't lost. Recorders that don't
    /// buffer keep the default no-op.
    async fn flush(&mut self) {}
}
