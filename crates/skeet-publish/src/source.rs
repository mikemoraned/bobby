use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use backon::{ExponentialBuilder, Retryable};
use chrono::{DateTime, Utc};
use shared::SkeetId;
use skeet_store::StoreError;
use tracing::warn;

use crate::examined_count::ExaminedCount;
use crate::limit::Limit;
use crate::list_statistics::ListStatistics;
use crate::order::Order;
use crate::published::PublishedImage;
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

/// The full per-image published list in published order (not deduped to
/// skeet-ids, unlike [`FeedSkeleton`]) plus when it was last refreshed. Backs
/// the public image page.
pub struct PublishedImages {
    pub images: Vec<PublishedImage>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub statistics: Option<ListStatistics>,
}

/// Source of the full published image list.
///
/// Kept separate from [`FeedSource`] because the public page renders every
/// published image, not the skeet-id-deduped skeleton `getFeedSkeleton` returns.
#[async_trait]
pub trait PublishedImagesSource: Send + Sync {
    async fn published_images(&self) -> Result<PublishedImages, FeedSourceError>;

    /// The precalculated "images examined" count for the banner, or `None` if the
    /// publisher hasn't written it yet (a fresh deploy).
    async fn examined_count(&self) -> Result<Option<u64>, FeedSourceError>;
}

/// A redis access right after a Fly suspend/resume can hit a transient
/// connect/timeout/IO failure — DNS, the TLS handshake, or a socket Upstash
/// closed while the machine was idle. Those are worth a quick retry; a
/// malformed-JSON decode or any other logic error will only recur, so it isn't.
fn is_transient(e: &FeedSourceError) -> bool {
    match e {
        FeedSourceError::Published(PublishedListError::Redis(e)) => {
            e.is_connection_dropped()
                || e.is_connection_refusal()
                || e.is_timeout()
                || e.is_io_error()
        }
        _ => false,
    }
}

fn retry_policy() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(200))
        .with_factor(2.0)
        .with_max_times(3)
}

/// `FeedSource` backed by a published redis list on the publish server.
///
/// Reads the per-image `PublishedImage`s and dedups to a unique, order-preserving
/// list of skeet-ids — `getFeedSkeleton`'s view. The list already reflects the
/// publisher's policy (visibility, recency order, window), so `force_refresh` is
/// a no-op: there is nothing local to recompute.
///
/// Connects fresh per call (see [`crate::redis_client::connect`]) rather than
/// holding a pool, with a bounded retry on transient failures so the first read
/// after a suspend/resume re-establishes the connection (see [`is_transient`]).
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
    ) -> Result<(Vec<PublishedImage>, Option<DateTime<Utc>>), FeedSourceError> {
        (|| async {
            let mut conn = connect(&self.url).await.map_err(PublishedListError::from)?;
            Ok(self.list.read_with_refreshed_at(&mut conn).await?)
        })
        .retry(retry_policy())
        .when(is_transient)
        .notify(|e, dur| {
            warn!(
                error = %e,
                retry_in_ms = dur.as_millis() as u64,
                "transient redis failure reading published list; will retry",
            );
        })
        .await
    }

    /// This list's published statistics, or `None` if none have been written yet.
    pub async fn statistics(&self) -> Result<Option<ListStatistics>, FeedSourceError> {
        (|| async {
            let mut conn = connect(&self.url).await.map_err(PublishedListError::from)?;
            Ok(self.list.read_statistics(&mut conn).await?)
        })
        .retry(retry_policy())
        .when(is_transient)
        .notify(|e, dur| {
            warn!(
                error = %e,
                retry_in_ms = dur.as_millis() as u64,
                "transient redis failure reading list statistics; will retry",
            );
        })
        .await
    }
}

#[async_trait]
impl FeedSource for RedisFeedSource {
    async fn skeleton(&self, _force_refresh: bool) -> Result<FeedSkeleton, FeedSourceError> {
        let (published, refreshed_at) = self.published().await?;

        let mut seen = HashSet::new();
        let skeet_ids = published
            .into_iter()
            .filter(PublishedImage::is_live)
            .map(|item| item.skeet_id)
            .filter(|skeet_id| seen.insert(skeet_id.clone()))
            .collect();

        Ok(FeedSkeleton {
            skeet_ids,
            refreshed_at,
        })
    }
}

#[async_trait]
impl PublishedImagesSource for RedisFeedSource {
    async fn published_images(&self) -> Result<PublishedImages, FeedSourceError> {
        let (images, refreshed_at) = self.published().await?;
        let images = images.into_iter().filter(PublishedImage::is_live).collect();
        let statistics = self.statistics().await?;
        Ok(PublishedImages {
            images,
            refreshed_at,
            statistics,
        })
    }

    async fn examined_count(&self) -> Result<Option<u64>, FeedSourceError> {
        (|| async {
            let mut conn = connect(&self.url).await.map_err(PublishedListError::from)?;
            Ok(ExaminedCount::read(&mut conn).await?)
        })
        .retry(retry_policy())
        .when(is_transient)
        .notify(|e, dur| {
            warn!(
                error = %e,
                retry_in_ms = dur.as_millis() as u64,
                "transient redis failure reading examined count; will retry",
            );
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deadpool_redis::redis::{ErrorKind, RedisError};

    #[test]
    fn io_redis_error_is_transient() {
        let e = FeedSourceError::Published(PublishedListError::Redis(RedisError::from((
            ErrorKind::Io,
            "connection reset",
        ))));
        assert!(is_transient(&e));
    }

    #[test]
    fn json_error_is_not_transient() {
        let json_err = serde_json::from_str::<i32>("not a number").expect_err("should fail");
        let e = FeedSourceError::Published(PublishedListError::Json(json_err));
        assert!(!is_transient(&e));
    }

    #[test]
    fn non_transient_redis_error_is_not_retried() {
        let e = FeedSourceError::Published(PublishedListError::Redis(RedisError::from((
            ErrorKind::UnexpectedReturnType,
            "wrong type",
        ))));
        assert!(!is_transient(&e));
    }
}
