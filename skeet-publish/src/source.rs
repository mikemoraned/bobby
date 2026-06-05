use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use skeet_store::{SkeetId, StoreError};

use crate::limit::Limit;
use crate::order::Order;
use crate::published::Published;
use crate::published_list::{PublishedList, PublishedListError};
use crate::redis_client::connect;

/// The feed as seen by the Bluesky feed generator: an ordered, unique,
/// visibility-filtered list of skeet ids plus when the backing data was
/// last refreshed (used for the `last-modified` header).
pub struct FeedSkeleton {
    pub skeet_ids: Vec<SkeetId>,
    pub refreshed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum FeedSourceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Published(#[from] PublishedListError),
}

/// Source of the published feed skeleton.
///
/// The single narrow surface `skeet-feed`'s `getFeedSkeleton` depends on. The
/// `force_refresh` flag bypasses any caching to back `cache-control: no-cache`.
#[async_trait]
pub trait FeedSource: Send + Sync {
    async fn skeleton(&self, force_refresh: bool) -> Result<FeedSkeleton, FeedSourceError>;
}

/// `FeedSource` backed by a published redis list on the publish server.
///
/// Reads the per-image `Published`s and dedups to a unique, order-preserving
/// list of skeet-ids — `getFeedSkeleton`'s view. The list already reflects the
/// publisher's policy (visibility, recency order, window), so `force_refresh` is
/// a no-op: there is nothing local to recompute.
///
/// Connects fresh per call (see [`crate::redis_client::connect`]) rather than
/// holding a pool.
pub struct RedisFeedSource {
    url: String,
    list: PublishedList,
}

impl RedisFeedSource {
    pub fn new(url: impl Into<String>, order: Order, limit: Limit) -> Self {
        Self {
            url: url.into(),
            list: PublishedList::new(order, limit),
        }
    }

    pub async fn published(
        &self,
    ) -> Result<(Vec<Published>, Option<DateTime<Utc>>), FeedSourceError> {
        let mut conn = connect(&self.url).await.map_err(PublishedListError::from)?;
        let published = self.list.read(&mut conn).await?;
        let refreshed_at = self.list.refreshed_at(&mut conn).await?;
        Ok((published, refreshed_at))
    }
}

#[async_trait]
impl FeedSource for RedisFeedSource {
    async fn skeleton(&self, _force_refresh: bool) -> Result<FeedSkeleton, FeedSourceError> {
        let (published, refreshed_at) = self.published().await?;

        let mut seen = HashSet::new();
        let skeet_ids = published
            .into_iter()
            .map(|item| item.skeet_id)
            .filter(|skeet_id| seen.insert(skeet_id.clone()))
            .collect();

        Ok(FeedSkeleton {
            skeet_ids,
            refreshed_at,
        })
    }
}
