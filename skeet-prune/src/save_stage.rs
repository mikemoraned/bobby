use std::sync::Arc;

use skeet_store::SkeetStore;
use tokio::sync::mpsc;
use tracing::warn;

use crate::pipeline::{ChannelMonitors, ImageResult, PipelineCounters};
use crate::{persistence, status};

pub async fn run(
    rx: &mut mpsc::Receiver<ImageResult>,
    store: &SkeetStore,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
    log_interval: std::time::Duration,
) {
    let mut status = status::Status::new(log_interval, 100, counters, channels);

    while let Some(result) = rx.recv().await {
        if status.is_time_to_log()
            && let Ok(counts) = store.fragment_counts().await
        {
            status.update_fragment_counts(counts);
        }

        match result {
            ImageResult::Post { image_count } => {
                status.record_post(image_count);
            }
            ImageResult::Classified(record) => {
                persistence::save(store, &record, &mut status).await;
            }
            ImageResult::Rejected(reasons) => {
                status.record_rejected(&reasons);
            }
        }
    }

    warn!("filter stage ended, shutting down");
}
