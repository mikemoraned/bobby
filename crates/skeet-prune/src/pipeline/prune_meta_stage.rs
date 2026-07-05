use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_channel::{Receiver, Sender};
use bluesky::BlueskyError;
use serde_json::Value;
use shared::Rejection;
use shared::labels::ModerationLabel;
use tokio_util::sync::CancellationToken;
use tracing::{info, trace, warn};

use crate::firehose::SkeetCandidate;
use crate::pipeline::{self, ContentCounts, MetaMessage, MetaResult, PipelineCounters};

pub enum MetaFilterOutcome {
    Pass,
    /// Blocked by moderation labels on the post, its author, or a quoted record.
    Blocked(HashSet<ModerationLabel>),
    /// The `getPostThread` fetch failed; the post is rejected fail-closed.
    FetchFailed(BlueskyError),
}

/// Build the single meta→image message for a candidate from its metadata
/// outcome. A passed candidate contributes its full image count; a blocked one
/// contributes a post with no images plus the metadata rejection — both count as
/// one observed post, and the rejection is tallied here so nothing downstream
/// has to carry it.
fn meta_message(candidate: SkeetCandidate, outcome: MetaFilterOutcome) -> MetaMessage {
    match outcome {
        MetaFilterOutcome::Pass => {
            let images = candidate.images.len() as u64;
            (MetaResult::Candidate(candidate), ContentCounts::post(images))
        }
        MetaFilterOutcome::Blocked(_) | MetaFilterOutcome::FetchFailed(_) => (
            MetaResult::Rejected,
            ContentCounts::post(0) + ContentCounts::rejected(&[Rejection::BlockedByMetadata]),
        ),
    }
}

/// Check whether a skeet should be blocked based on its `getPostThread` metadata.
///
/// Inspects post labels, author labels, and quoted-record author labels for
/// excluded values (adult content, `!no-unauthenticated`, etc.).
pub fn check_metadata(post_thread_json: &Value) -> MetaFilterOutcome {
    let blocked = bluesky::blocked_labels(post_thread_json);
    if blocked.is_empty() {
        MetaFilterOutcome::Pass
    } else {
        MetaFilterOutcome::Blocked(blocked)
    }
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
            Err(e) => MetaFilterOutcome::FetchFailed(e),
        };
        match &outcome {
            MetaFilterOutcome::Blocked(labels) => {
                let labels = labels
                    .iter()
                    .map(ModerationLabel::as_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                trace!(skeet_id = %candidate.skeet_id, %labels, "blocked by moderation labels, rejecting");
            }
            MetaFilterOutcome::FetchFailed(err) => {
                trace!(skeet_id = %candidate.skeet_id, %err, "getPostThread fetch failed, rejecting");
            }
            MetaFilterOutcome::Pass => {}
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
                url: bluesky::ImageUrl::new("https://example.com/img").expect("valid url"),
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
    fn blocked_tallies_the_rejection_into_its_counts() {
        let (result, counts) = meta_message(
            candidate_with_images(3),
            MetaFilterOutcome::Blocked(HashSet::from([ModerationLabel::new("porn")])),
        );
        assert!(matches!(result, MetaResult::Rejected));
        assert_eq!(
            counts,
            ContentCounts::post(0) + ContentCounts::rejected(&[Rejection::BlockedByMetadata])
        );
    }
}
