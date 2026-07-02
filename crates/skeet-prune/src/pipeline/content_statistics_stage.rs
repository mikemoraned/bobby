use std::sync::Arc;

use async_channel::Receiver;
use skeet_store::Statistics;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::pipeline::content_counts_recorder::ContentCountsRecorder;
use crate::pipeline::statistics_persister::StatisticsPersister;
use crate::pipeline::{self, ChannelMonitors, PipelineCounters, StatsMessage};
use crate::status::Status;

/// Final stage: fan each message's `ContentCounts` out to the recorders.
///
/// [`Status`] handles cumulative logging and metrics; [`StatisticsPersister`]
/// writes per-interval [`PruneStats`](skeet_store::PruneStats) to the store.
/// The stage does no save work — the save stage upstream has already folded its
/// `saved` decisions into the counts that arrive here.
pub async fn run(
    rx: &Receiver<StatsMessage>,
    statistics: &impl Statistics,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
    log_interval: std::time::Duration,
    flush_interval: std::time::Duration,
    token: CancellationToken,
) {
    let mut recorders: Vec<Box<dyn ContentCountsRecorder + Send + '_>> = vec![
        Box::new(Status::new(log_interval, 100, counters, channels)),
        Box::new(StatisticsPersister::new(
            statistics,
            log_interval,
            flush_interval,
        )),
    ];

    while let Some(counts) = pipeline::recv(rx, &token).await {
        for recorder in &mut recorders {
            recorder.record_counts(&counts).await;
        }
    }

    for recorder in &mut recorders {
        recorder.flush().await;
    }

    warn!("content statistics stage ended, shutting down");
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use shared::Rejection;
    use skeet_store::test_utils::open_temp_store;
    use test_support::flush_and_collect;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::pipeline::{ContentCounts, ImageMessage, MetaMessage};

    fn dummy_channels() -> ChannelMonitors {
        let (firehose_tx, _f) = async_channel::bounded(1);
        let (meta_tx, _m) = async_channel::bounded::<MetaMessage>(1);
        let (image_tx, _i) = async_channel::bounded::<ImageMessage>(1);
        ChannelMonitors::new(firehose_tx, meta_tx, image_tx)
    }

    /// The fixed stream of per-candidate `ContentCounts` the golden test merges:
    /// rejections and `saved` already folded in (as the stages do upstream). The
    /// asserted totals below stay byte-identical across counting-shape changes.
    fn scenario() -> Vec<ContentCounts> {
        use Rejection::{BlockedByMetadata, FaceTooSmall, TooMuchText};
        let reject = |reason| ContentCounts::rejected(&[reason]);
        vec![
            // A: meta-rejected (no images examined).
            ContentCounts::post(0) + reject(BlockedByMetadata),
            // B: passed, 3 images, one fresh save, two rejections of varied category.
            ContentCounts::post(3) + ContentCounts::saved() + reject(FaceTooSmall) + reject(TooMuchText),
            // C: passed, 2 images, all reject.
            ContentCounts::post(2) + reject(FaceTooSmall) + reject(FaceTooSmall),
            // D: passed, 1 image, fresh save.
            ContentCounts::post(1) + ContentCounts::saved(),
            // E: passed, 1 image, already existed (no save).
            ContentCounts::post(1),
        ]
    }

    #[tokio::test]
    async fn records_message_counts_as_metrics() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        opentelemetry::global::set_meter_provider(provider.clone());

        let (tx, rx) = async_channel::bounded(64);
        for counts in scenario() {
            tx.send(counts).await.expect("send scenario message");
        }
        // Close the channel so the stage's receive loop ends and `run` returns.
        drop(tx);

        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;
        let before = Utc::now();

        let counters = Arc::new(PipelineCounters::default());
        // A zero interval makes every post-bearing message flush a summary, so the
        // final emit captures the grand totals regardless of the every-N cadence.
        run(
            &rx,
            &store,
            counters,
            dummy_channels(),
            Duration::ZERO,
            Duration::ZERO,
            CancellationToken::new(),
        )
        .await;

        // The per-interval PruneStats recorded across the run sum to the grand
        // totals (skeets/images/saved map 1:1 to the metric counters below).
        let recorded = store
            .prune_stats_for_interval(before, Utc::now())
            .await
            .expect("interval counts");
        assert_eq!(recorded.skeets_seen, 5);
        assert_eq!(recorded.images_examined, 7);
        assert_eq!(recorded.images_saved, 2);

        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(snap.sum_counter("skeet_prune.skeets.total", None), 5);
        assert_eq!(snap.sum_counter("skeet_prune.images.total", None), 7);
        assert_eq!(snap.sum_counter("skeet_prune.saved.total", None), 2);

        let rejected =
            |reason: &str| snap.sum_counter("skeet_prune.rejected.total", Some(("reason", reason)));
        assert_eq!(rejected("BlockedByMetadata"), 1);
        assert_eq!(rejected("FaceTooSmall"), 3);
        assert_eq!(rejected("TooMuchText"), 1);

        let category =
            |cat: &str| snap.sum_counter("skeet_prune.categories.total", Some(("category", cat)));
        assert_eq!(category("Metadata"), 1);
        assert_eq!(category("Face"), 3);
        assert_eq!(category("Text"), 1);

        let sole = |cat: &str| {
            snap.sum_counter("skeet_prune.categories.sole.total", Some(("category", cat)))
        };
        assert_eq!(sole("Metadata"), 1);
        assert_eq!(sole("Face"), 3);
        assert_eq!(sole("Text"), 1);
    }
}
