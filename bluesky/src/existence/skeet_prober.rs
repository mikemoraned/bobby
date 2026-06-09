use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use shared::skeet_id::SkeetId;
use tracing::warn;

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
/// One `getPostThread` per skeet yields both availability (a deleted/blocked post
/// comes back as a `notFoundPost`/`blockedPost` node) and the moderation labels,
/// so the publisher applies the same exclusion the firehose pruner does.
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
        let mut out = HashMap::with_capacity(skeets.len());
        let mut pending = skeets.iter().cloned();
        let mut in_flight = tokio::task::JoinSet::new();

        let spawn = |set: &mut tokio::task::JoinSet<(SkeetId, bool)>, skeet: SkeetId| {
            let client = self.client.clone();
            set.spawn(async move {
                let exists = probe_one_skeet(&client, &skeet).await;
                (skeet, exists)
            });
        };

        for skeet in pending.by_ref().take(self.concurrency) {
            spawn(&mut in_flight, skeet);
        }
        while let Some(result) = in_flight.join_next().await {
            if let Ok((skeet, exists)) = result {
                out.insert(skeet, exists);
            }
            if let Some(skeet) = pending.next() {
                spawn(&mut in_flight, skeet);
            }
        }
        out
    }
}

/// Fetch a skeet's post thread and decide whether it should still be shown.
/// Fail-open: a request error (transport, timeout, server error) is inconclusive
/// and reported as still present.
async fn probe_one_skeet(client: &reqwest::Client, skeet: &SkeetId) -> bool {
    match crate::fetch_post_thread(client, skeet).await {
        Ok(json) => is_viewable(&json),
        Err(e) => {
            warn!(skeet = %skeet, error = %e, "skeet probe failed; treating as still present");
            true
        }
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
}
