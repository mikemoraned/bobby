use std::sync::Arc;

use skeet_store::SkeetStore;
use tokio::sync::mpsc;
use tracing::warn;

use crate::pipeline::{ChannelMonitors, ImageResult, PipelineCounters};
use crate::{persistence, status};

pub async fn run(
    rx: &mut mpsc::Receiver<ImageResult>,
    store: &SkeetStore,
    fallback: Option<&SkeetStore>,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
    log_interval: std::time::Duration,
) {
    let mut status = status::Status::new(log_interval, 100, counters, channels);

    while let Some(result) = rx.recv().await {
        match result {
            ImageResult::Post { image_count } => {
                status.record_post(image_count);
            }
            ImageResult::Classified(record) => {
                if let Some(fallback_store) = fallback {
                    persistence::save_with_fallback(
                        store,
                        fallback_store,
                        &record,
                        &mut status,
                    )
                    .await;
                } else {
                    persistence::save(store, &record, &mut status).await;
                }
            }
            ImageResult::Rejected(reasons) => {
                status.record_rejected(&reasons);
            }
        }
    }

    warn!("filter stage ended, shutting down");
}
