use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use skeet_store::{SkeetId, StoreError};

use crate::feed_cache::FeedCache;
use crate::visibility::visible_entries;

/// The feed as seen by the Bluesky feed generator: an ordered, unique,
/// visibility-filtered list of skeet ids plus when the backing data was
/// last refreshed (used for the `last-modified` header).
pub struct FeedSkeleton {
    pub skeet_ids: Vec<SkeetId>,
    pub refreshed_at: Option<DateTime<Utc>>,
}

/// Source of the published feed skeleton.
///
/// The single narrow surface `skeet-feed`'s `getFeedSkeleton` depends on. The
/// `force_refresh` flag bypasses any caching to back `cache-control: no-cache`.
#[async_trait]
pub trait FeedSource: Send + Sync {
    async fn skeleton(&self, force_refresh: bool) -> Result<FeedSkeleton, StoreError>;
}

/// `FeedSource` backed by the live store via `FeedCache`, applying the
/// visibility/scoring policy in `visible_entries`.
pub struct LiveFeedSource {
    cache: Arc<FeedCache>,
}

impl LiveFeedSource {
    pub const fn new(cache: Arc<FeedCache>) -> Self {
        Self { cache }
    }
}

#[async_trait]
impl FeedSource for LiveFeedSource {
    async fn skeleton(&self, force_refresh: bool) -> Result<FeedSkeleton, StoreError> {
        let feed = if force_refresh {
            self.cache.refresh().await?
        } else {
            self.cache.get().await?
        };
        let skeet_ids = visible_entries(&feed)
            .into_iter()
            .map(|(summary, _score, _model_version)| summary.skeet_id)
            .collect();
        Ok(FeedSkeleton {
            skeet_ids,
            refreshed_at: self.cache.refreshed_at().await,
        })
    }
}
