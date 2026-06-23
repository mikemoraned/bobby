use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::firehose::SkeetCandidate;
use crate::pipeline::{self, PipelineCounters};

pub async fn run(
    tx: mpsc::Sender<SkeetCandidate>,
    counters: Arc<PipelineCounters>,
    token: CancellationToken,
) {
    let recv_timeout = std::time::Duration::from_secs(30);

    // The resume cursor lives only in memory: it survives reconnects within
    // this process, but a restart resumes at live-tail (the redeploy/crash gap
    // is intentionally accepted as lost).
    let mut last_time_us: Option<u64> = None;

    loop {
        let cursor = last_time_us.and_then(crate::firehose::cursor_from);
        let receiver = match crate::firehose::connect(cursor).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "failed to connect to firehose, retrying");
                continue;
            }
        };
        info!("firehose connected, listening for posts...");

        loop {
            let event = tokio::select! {
                () = token.cancelled() => return,
                result = tokio::time::timeout(recv_timeout, receiver.recv_async()) => match result {
                    Ok(Ok(event)) => event,
                    Ok(Err(_)) => {
                        warn!("firehose channel closed");
                        break;
                    }
                    Err(_) => {
                        warn!("no message received in {recv_timeout:?}, reconnecting");
                        break;
                    }
                },
            };

            last_time_us = Some(crate::firehose::event_time_us(&event));

            if let Some(candidate) = crate::firehose::extract_skeet_candidate(&event) {
                counters.firehose.fetch_add(1, Ordering::Relaxed);
                if pipeline::forward(&tx, candidate, &token).await.is_err() {
                    return;
                }
            }
        }
    }
}
