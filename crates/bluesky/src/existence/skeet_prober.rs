use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::Value;
use shared::skeet_id::SkeetId;
use tracing::warn;

use crate::BlueskyError;

/// Checks whether skeets still exist on Bluesky.
///
/// Split from image probing (which does an unrelated job over a different host)
/// so each can be implemented and faked independently.
#[async_trait]
pub trait SkeetProber: Send + Sync {
    /// Whether each skeet currently exists. Fail-open: an inconclusive check
    /// reports the skeet as still present.
    async fn probe_skeets(&self, skeets: &[SkeetId]) -> HashMap<SkeetId, bool>;
}

/// Probes the real Bluesky public API via `app.bsky.feed.getPostThread`, fetching
/// at most `concurrency` posts at once.
///
/// One `getPostThread` per skeet yields availability and the moderation labels in
/// one call, so the publisher applies the same exclusion the firehose pruner
/// does. Absence shows up two ways: a hard error status (a deleted post returns
/// `400 NotFound`; see [`gone_on_error`]) or a `200` carrying a
/// `notFoundPost`/`blockedPost` thread node (see [`is_viewable`]).
pub struct CdnSkeetProber {
    client: reqwest::Client,
    concurrency: usize,
}

impl CdnSkeetProber {
    pub fn new(concurrency: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client,
            concurrency: concurrency.max(1),
        }
    }
}

#[async_trait]
impl SkeetProber for CdnSkeetProber {
    async fn probe_skeets(&self, skeets: &[SkeetId]) -> HashMap<SkeetId, bool> {
        let client = self.client.clone();
        super::probe_bounded(skeets, self.concurrency, move |skeet| {
            let client = client.clone();
            async move {
                let exists = probe_one_skeet(&client, &skeet).await;
                (skeet, exists)
            }
        })
        .await
    }
}

/// Fetch a skeet's post thread and decide whether it should still be shown.
///
/// A definitive "gone" failure (see [`gone_on_error`]) reports the skeet as
/// absent; any other failure (transport, timeout, rate-limit, server error) is
/// inconclusive and fails open to still-present.
async fn probe_one_skeet(client: &reqwest::Client, skeet: &SkeetId) -> bool {
    match crate::fetch_post_thread(client, skeet).await {
        Ok(json) => is_viewable(&json),
        Err(e) if gone_on_error(&e) => false,
        Err(e) => {
            warn!(skeet = %skeet, error = %e, "skeet probe inconclusive; treating as still present");
            true
        }
    }
}

/// Whether a failed `getPostThread` means the post is genuinely gone, as opposed
/// to an inconclusive failure we should fail open on.
///
/// The public AppView answers a deleted/missing post with a client error — a
/// `400 NotFound` for a deleted post, and `404`/`410` for other not-there cases
/// — so any `4xx` is treated as gone, except a rate-limit (`429`). A `5xx` server
/// error or a transport failure is inconclusive and leaves the post present.
fn gone_on_error(error: &BlueskyError) -> bool {
    match error {
        BlueskyError::Status { status, .. } => {
            status.is_client_error() && *status != StatusCode::TOO_MANY_REQUESTS
        }
        BlueskyError::Request(_) => false,
    }
}

/// Whether a post thread is a real, viewable post carrying no excluded
/// moderation label — the same exclusion the firehose pruner applies at ingest.
fn is_viewable(post_thread_json: &Value) -> bool {
    crate::post_is_available(post_thread_json) && crate::blocked_labels(post_thread_json).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_post_is_viewable() {
        let json = serde_json::json!({
            "thread": {
                "$type": "app.bsky.feed.defs#threadViewPost",
                "post": { "author": { "labels": [] }, "labels": [] }
            }
        });
        assert!(is_viewable(&json));
    }

    #[test]
    fn deleted_post_is_not_viewable() {
        let json = serde_json::json!({
            "thread": { "$type": "app.bsky.feed.defs#notFoundPost", "notFound": true }
        });
        assert!(!is_viewable(&json));
    }

    #[test]
    fn adult_labelled_post_is_not_viewable() {
        let json = serde_json::json!({
            "thread": {
                "$type": "app.bsky.feed.defs#threadViewPost",
                "post": { "labels": [ { "val": "porn", "src": "did:plc:x" } ] }
            }
        });
        assert!(!is_viewable(&json));
    }

    fn status_error(status: StatusCode) -> BlueskyError {
        BlueskyError::Status {
            status,
            body: String::new(),
        }
    }

    #[test]
    fn client_errors_mean_gone() {
        // A deleted post comes back as `400 NotFound` from the public AppView.
        assert!(gone_on_error(&status_error(StatusCode::BAD_REQUEST)));
        assert!(gone_on_error(&status_error(StatusCode::NOT_FOUND)));
        assert!(gone_on_error(&status_error(StatusCode::GONE)));
    }

    #[test]
    fn rate_limit_and_server_errors_are_inconclusive() {
        assert!(!gone_on_error(&status_error(StatusCode::TOO_MANY_REQUESTS)));
        assert!(!gone_on_error(&status_error(
            StatusCode::INTERNAL_SERVER_ERROR
        )));
        assert!(!gone_on_error(&status_error(StatusCode::BAD_GATEWAY)));
    }
}
