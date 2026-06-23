use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;
use shared::Rejection;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::trace;

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

/// Pipeline stage: receives candidates from `firehose_stage`, fetches post
/// metadata, and forwards only those that pass the metadata check.
pub async fn run(
    rx: &mut mpsc::Receiver<SkeetCandidate>,
    tx: mpsc::Sender<MetaResult>,
    http: reqwest::Client,
    counters: Arc<PipelineCounters>,
    token: CancellationToken,
) {
    while let Some(candidate) = pipeline::recv(rx, &token).await {
        counters.meta.fetch_add(1, Ordering::Relaxed);
        let image_count = candidate.images.len() as u64;

        let (result, passed) = match bluesky::fetch_post_thread(&http, &candidate.skeet_id).await {
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

        if pipeline::forward(&tx, result, &token).await.is_err() {
            return;
        }

        let image_count = if passed { image_count } else { 0 };
        if pipeline::forward(&tx, MetaResult::Post { image_count }, &token)
            .await
            .is_err()
        {
            return;
        }
    }
}
