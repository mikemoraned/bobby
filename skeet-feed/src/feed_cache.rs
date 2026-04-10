use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use skeet_store::{Score, SkeetStore, StoredImageSummary};
use tokio::sync::RwLock;
use tokio::time::Instant;
use tower::{Layer, Service};
use tracing::{info, warn};

/// Maximum age of cached data before a request triggers a synchronous refresh.
const MAX_CACHE_STALENESS: Duration = Duration::from_secs(5 * 60);

/// How often the background worker refreshes the cache.
const BACKGROUND_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

type FeedEntries = Vec<(StoredImageSummary, Score)>;

struct CacheEntry {
    entries: FeedEntries,
    fetched_at: Instant,
}

pub struct FeedCache {
    store: Arc<SkeetStore>,
    limit: usize,
    max_age_hours: u64,
    inner: RwLock<Option<CacheEntry>>,
}

impl FeedCache {
    pub fn new(store: Arc<SkeetStore>, limit: usize, max_age_hours: u64) -> Self {
        Self {
            store,
            limit,
            max_age_hours,
            inner: RwLock::new(None),
        }
    }

    pub async fn get(&self) -> Result<FeedEntries, skeet_store::StoreError> {
        {
            let guard = self.inner.read().await;
            if let Some(entry) = guard.as_ref() {
                let staleness = entry.fetched_at.elapsed();
                if staleness < MAX_CACHE_STALENESS {
                    info!(staleness_secs = staleness.as_secs(), "serving from cache");
                    return Ok(entry.entries.clone());
                }
            }
        }

        self.refresh().await
    }

    pub async fn refresh(&self) -> Result<FeedEntries, skeet_store::StoreError> {
        let entries = self
            .store
            .list_scored_summaries_by_score(self.limit, Some(self.max_age_hours))
            .await?;

        let cloned = entries.clone();
        {
            let mut guard = self.inner.write().await;
            *guard = Some(CacheEntry {
                entries,
                fetched_at: Instant::now(),
            });
        }

        Ok(cloned)
    }

    pub fn spawn_background_refresh(self: &Arc<Self>) {
        let cache = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(BACKGROUND_REFRESH_INTERVAL).await;
                match cache.refresh().await {
                    Ok(entries) => {
                        info!(count = entries.len(), "background cache refresh complete");
                    }
                    Err(e) => {
                        warn!(error = %e, "background cache refresh failed");
                    }
                }
            }
        });
    }

    pub async fn staleness(&self) -> Option<Duration> {
        let guard = self.inner.read().await;
        guard.as_ref().map(|entry| entry.fetched_at.elapsed())
    }
}

#[derive(Clone)]
pub struct FeedCacheExtractor(pub Arc<FeedCache>);

impl FromRequestHead for FeedCacheExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<FeedCache>>()
            .cloned()
            .map(FeedCacheExtractor)
            .ok_or_else(|| cot::Error::internal("FeedCache not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct FeedCacheLayer {
    cache: Arc<FeedCache>,
}

impl FeedCacheLayer {
    pub const fn new(cache: Arc<FeedCache>) -> Self {
        Self { cache }
    }
}

impl<S> Layer<S> for FeedCacheLayer {
    type Service = FeedCacheService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FeedCacheService {
            inner,
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct FeedCacheService<S> {
    inner: S,
    cache: Arc<FeedCache>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for FeedCacheService<S>
where
    S: Service<cot::http::Request<ReqBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: cot::http::Request<ReqBody>) -> Self::Future {
        req.extensions_mut().insert(self.cache.clone());
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skeet_store::test_utils::{make_record, open_temp_store};
    use skeet_store::ModelVersion;

    async fn seed_store(store: &SkeetStore, suffix: &str, r: u8, score: f32) {
        let record = make_record(suffix, r, 0, 0);
        let image_id = record.image_id.clone();
        store.add(&record).await.expect("add record");
        store
            .upsert_score(
                &image_id,
                &Score::new(score).expect("valid score"),
                &ModelVersion::from("test"),
            )
            .await
            .expect("upsert score");
    }

    #[tokio::test]
    async fn cache_miss_populates_on_first_get() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), 10, 48);
        let result = cache.get().await.expect("get");
        assert_eq!(result.len(), 1);
        assert!(cache.staleness().await.is_some());
    }

    /// Populates a cache with one record, adds a second record, then advances
    /// time by `advance_by`. Returns the length of the result from `get()` after
    /// the time advance.
    async fn cache_get_len_after_advance(advance_by: Duration) -> usize {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), 10, 48);
        cache.get().await.expect("first get");

        seed_store(&store, "b", 20, 0.8).await;
        tokio::time::advance(advance_by).await;

        cache.get().await.expect("second get").len()
    }

    #[tokio::test(start_paused = true)]
    async fn cache_hit_returns_stale_data_within_window() {
        let len = cache_get_len_after_advance(MAX_CACHE_STALENESS - Duration::from_secs(1)).await;
        assert_eq!(len, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cache_hit_refetches_data_outwith_window() {
        let len = cache_get_len_after_advance(MAX_CACHE_STALENESS + Duration::from_secs(1)).await;
        assert_eq!(len, 2);
    }

    #[tokio::test]
    async fn refresh_picks_up_new_data() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), 10, 48);
        let first = cache.get().await.expect("first get");
        assert_eq!(first.len(), 1);

        seed_store(&store, "b", 20, 0.8).await;
        let refreshed = cache.refresh().await.expect("refresh");
        assert_eq!(refreshed.len(), 2);

        // subsequent get should return refreshed data
        let after = cache.get().await.expect("get after refresh");
        assert_eq!(after.len(), 2);
    }

    #[tokio::test]
    async fn empty_store_returns_empty_cache() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);

        let cache = FeedCache::new(Arc::clone(&store), 10, 48);
        let result = cache.get().await.expect("get");
        assert!(result.is_empty());
    }
}
