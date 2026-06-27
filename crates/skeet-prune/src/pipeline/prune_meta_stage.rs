use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_channel::{Receiver, Sender};
use serde_json::Value;
use shared::Rejection;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};

use crate::firehose::SkeetCandidate;
use crate::pipeline::{self, ContentCounts, MetaMessage, MetaResult, PipelineCounters};

pub enum MetaFilterOutcome {
    Pass,
    /// Blocked, carrying a human-readable reason for the trace log. A failed
    /// `getPostThread` fetch is treated the same way.
    Blocked(String),
}

/// Build the single meta→image message for a candidate from its metadata
/// outcome. A passed candidate contributes its full image count; a blocked one
/// contributes none — both count as one observed post.
fn meta_message(candidate: SkeetCandidate, outcome: MetaFilterOutcome) -> MetaMessage {
    match outcome {
        MetaFilterOutcome::Pass => {
            let images = candidate.images.len() as u64;
            (MetaResult::Candidate(candidate), ContentCounts::post(images))
        }
        MetaFilterOutcome::Blocked(_) => (
            MetaResult::Rejected(vec![Rejection::BlockedByMetadata]),
            ContentCounts::post(0),
        ),
    }
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
    tx: Sender<MetaMessage>,
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
    tx: &Sender<MetaMessage>,
    http: &reqwest::Client,
    counters: &PipelineCounters,
    token: &CancellationToken,
) {
    while let Some(candidate) = pipeline::recv(rx, token).await {
        counters.meta.fetch_add(1, Ordering::Relaxed);

        let outcome = match bluesky::fetch_post_thread(http, &candidate.skeet_id).await {
            Ok(json) => check_metadata(&json),
            Err(e) => MetaFilterOutcome::Blocked(format!("fetch failed: {e}")),
        };
        if let MetaFilterOutcome::Blocked(reason) = &outcome {
            trace!(skeet_id = %candidate.skeet_id, reason, "blocked by metadata, rejecting");
        }

        if pipeline::forward(tx, meta_message(candidate, outcome), token)
            .await
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use shared::BlueskyCid;

    use super::*;
    use crate::firehose::ImageCandidate;

    const VALID_CID: &str = "bafyreibvjvcv745gig4mvqs4hctx4zfkono4rjejm2ta6gtyay3sefw7p4";

    fn candidate_with_images(n: usize) -> SkeetCandidate {
        let images = (0..n)
            .map(|_| ImageCandidate {
                cid: BlueskyCid::new(VALID_CID).expect("valid cid"),
                url: "https://example.com/img".to_string(),
            })
            .collect();
        SkeetCandidate {
            skeet_id: "at://did:plc:abc/app.bsky.feed.post/abc"
                .parse()
                .expect("valid skeet id"),
            original_at: Utc::now(),
            images,
        }
    }

    #[test]
    fn pass_carries_one_post_and_full_image_count() {
        let (result, counts) = meta_message(candidate_with_images(3), MetaFilterOutcome::Pass);
        assert!(matches!(result, MetaResult::Candidate(_)));
        assert_eq!(counts, ContentCounts::post(3));
    }

    #[test]
    fn blocked_carries_one_post_and_no_images() {
        let (result, counts) = meta_message(
            candidate_with_images(3),
            MetaFilterOutcome::Blocked("blocked labels: porn".to_string()),
        );
        assert!(matches!(result, MetaResult::Rejected(_)));
        assert_eq!(counts, ContentCounts::post(0));
    }
}
