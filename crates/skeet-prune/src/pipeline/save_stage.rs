use std::sync::Arc;

use async_channel::Receiver;
use skeet_store::Images;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::pipeline::{self, ChannelMonitors, ImageMessage, ImageResult, PipelineCounters};
use crate::{persistence, status};

pub async fn run(
    rx: &Receiver<ImageMessage>,
    store: &impl Images,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
    log_interval: std::time::Duration,
    token: CancellationToken,
) {
    let mut status = status::Status::new(log_interval, 100, counters, channels);

    while let Some((images, counts)) = pipeline::recv(rx, &token).await {
        for image in images {
            match image {
                ImageResult::Classified(record) => {
                    persistence::save(store, &record, &mut status).await;
                }
                ImageResult::Rejected(reasons) => {
                    status.record_rejected(&reasons);
                }
            }
        }
        status.record_counts(&counts);
    }

    warn!("filter stage ended, shutting down");
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use shared::Rejection;
    use skeet_store::ImageRecord;
    use skeet_store::test_utils::{make_record, open_temp_store};
    use test_support::flush_and_collect;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::pipeline::{ContentCounts, MetaMessage};

    fn dummy_channels() -> ChannelMonitors {
        let (firehose_tx, _f) = async_channel::bounded(1);
        let (meta_tx, _m) = async_channel::bounded::<MetaMessage>(1);
        let (image_tx, _i) = async_channel::bounded::<ImageMessage>(1);
        ChannelMonitors::new(firehose_tx, meta_tx, image_tx)
    }

    /// The fixed message stream the golden test drives through the sink: one
    /// bundled `(images, counts)` message per candidate. The asserted totals
    /// below stay byte-identical across counting-shape changes.
    fn scenario(rec_b1: ImageRecord, rec_d: ImageRecord, rec_e: ImageRecord) -> Vec<ImageMessage> {
        use Rejection::{BlockedByMetadata, FaceTooSmall, TooMuchText};
        let reject = |reason| ImageResult::Rejected(vec![reason]);
        let classified = |rec| ImageResult::Classified(Box::new(rec));
        vec![
            // B: passed, 3 images (one fresh save, two rejections of varied category).
            (
                vec![classified(rec_b1), reject(FaceTooSmall), reject(TooMuchText)],
                ContentCounts::post(3),
            ),
            // C: passed, 2 images, all reject.
            (
                vec![reject(FaceTooSmall), reject(FaceTooSmall)],
                ContentCounts::post(2),
            ),
            // A: meta-rejected (no images examined).
            (vec![reject(BlockedByMetadata)], ContentCounts::post(0)),
            // D: passed, 1 image, fresh save.
            (vec![classified(rec_d)], ContentCounts::post(1)),
            // E: passed, 1 image, already-exists save (pre-seeded into the store).
            (vec![classified(rec_e)], ContentCounts::post(1)),
        ]
    }

    #[tokio::test]
    async fn sink_counts_match_golden_totals() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        opentelemetry::global::set_meter_provider(provider.clone());

        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;

        let rec_b1 = make_record("b1", 1, 0, 0);
        let rec_d = make_record("d", 2, 0, 0);
        let rec_e = make_record("e", 3, 0, 0);
        store.add(&rec_e).await.expect("seed already-exists record");

        let (tx, rx) = async_channel::bounded(64);
        for msg in scenario(rec_b1, rec_d, rec_e) {
            tx.send(msg).await.expect("send scenario message");
        }
        drop(tx);

        let counters = Arc::new(PipelineCounters::default());
        // A zero interval makes every post-bearing message flush a summary, so the
        // final emit captures the grand totals regardless of the every-N cadence.
        run(
            &rx,
            &store,
            counters,
            dummy_channels(),
            Duration::ZERO,
            CancellationToken::new(),
        )
        .await;

        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(snap.sum_counter("skeet_prune.skeets.total", None), 5);
        assert_eq!(snap.sum_counter("skeet_prune.images.total", None), 7);
        assert_eq!(snap.sum_counter("skeet_prune.saved.total", None), 2);

        let rejected = |reason: &str| {
            snap.sum_counter("skeet_prune.rejected.total", Some(("reason", reason)))
        };
        assert_eq!(rejected("BlockedByMetadata"), 1);
        assert_eq!(rejected("FaceTooSmall"), 3);
        assert_eq!(rejected("TooMuchText"), 1);

        let category = |cat: &str| {
            snap.sum_counter("skeet_prune.categories.total", Some(("category", cat)))
        };
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
