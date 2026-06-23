use std::sync::Arc;

use skeet_store::Images;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::pipeline::{self, ChannelMonitors, ImageResult, PipelineCounters};
use crate::{persistence, status};

pub async fn run(
    rx: &mut mpsc::Receiver<ImageResult>,
    store: &impl Images,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
    log_interval: std::time::Duration,
    token: CancellationToken,
) {
    let mut status = status::Status::new(log_interval, 100, counters, channels);

    while let Some(result) = pipeline::recv(rx, &token).await {
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
