use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_channel::{Receiver, Sender};
use serde_json::Value;
use shared::Rejection;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};

use crate::firehose::SkeetCandidate;
use crate::pipeline::{self, MetaResult, PipelineCounters};

pub enum MetaFilterOutcome {
    Pass,
    Blocked(String),
}

/// Check whether a skeet should be blocked based on its `getPostThread` metadata.
///
/// Inspects post labels, author labels, and quoted-record author labels for
/// excluded values (adult content, `!no-unauthenticated`, etc.).
pub fn check_metadata(post_thread_json: &Value) -> MetaFilterOutcome {
    let blocked = bluesky::blocked_labels(post_thread_json);
    if !blocked.is_empty() {
        return MetaFilterOutcome::Blocked(format!("blocked labels: {}", blocked.join(", ")));
    }

    MetaFilterOutcome::Pass
}

/// Pipeline stage: forward only candidates that pass the metadata check.
///
/// A pool of workers receive candidates from `firehose_stage`, fetch post
/// metadata, and forward those that pass. The stage is network-I/O-bound (one
/// serial `getPostThread` round-trip per candidate), so workers share the input
/// channel and run their fetches concurrently to keep pace with firehose intake.
pub async fn run_workers(
    rx: Receiver<SkeetCandidate>,
    tx: Sender<MetaResult>,
    http: reqwest::Client,
    counters: Arc<PipelineCounters>,
    num_workers: usize,
    token: CancellationToken,
) {
    info!(num_workers, "starting meta stage workers");

    let mut handles = Vec::with_capacity(num_workers);
    for _ in 0..num_workers {
        let rx = rx.clone();
        let tx = tx.clone();
        let http = http.clone();
        let counters = Arc::clone(&counters);
        let token = token.clone();
        handles.push(tokio::spawn(async move {
            run_single(&rx, &tx, &http, &counters, &token).await;
        }));
    }

    for handle in handles {
        if let Err(e) = handle.await {
            warn!("meta worker panicked: {e}");
        }
    }
}

async fn run_single(
    rx: &Receiver<SkeetCandidate>,
    tx: &Sender<MetaResult>,
    http: &reqwest::Client,
    counters: &PipelineCounters,
    token: &CancellationToken,
) {
    while let Some(candidate) = pipeline::recv(rx, token).await {
        counters.meta.fetch_add(1, Ordering::Relaxed);
        let image_count = candidate.images.len() as u64;

        let (result, passed) = match bluesky::fetch_post_thread(http, &candidate.skeet_id).await {
            Ok(json) => match check_metadata(&json) {
                MetaFilterOutcome::Pass => (MetaResult::Candidate(candidate), true),
                MetaFilterOutcome::Blocked(reason) => {
                    trace!(skeet_id = %candidate.skeet_id, reason, "blocked by metadata");
                    (
                        MetaResult::Rejected(vec![Rejection::BlockedByMetadata]),
                        false,
                    )
                }
            },
            Err(e) => {
                trace!(skeet_id = %candidate.skeet_id, error = %e, "failed to fetch post metadata, rejecting");
                (
                    MetaResult::Rejected(vec![Rejection::BlockedByMetadata]),
                    false,
                )
            }
        };

        if pipeline::forward(tx, result, token).await.is_err() {
            return;
        }

        let image_count = if passed { image_count } else { 0 };
        if pipeline::forward(tx, MetaResult::Post { image_count }, token)
            .await
            .is_err()
        {
            return;
        }
    }
}
