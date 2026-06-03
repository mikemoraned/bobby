use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use shared::{Band, ImageId, RefineModels};
use skeet_store::{
    Appraisal, ModelVersion, Score, SkeetId, SkeetStore, StoredImageSummary, Version,
};

use crate::table_watch::relevant;
use crate::visibility::FeedData;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// How often the background worker checks for version changes.
const BACKGROUND_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Cached feed data including scored summaries, manual appraisals, and the
/// model registry used to interpret each score's `model_version`.
#[derive(Clone)]
pub struct CachedFeed {
    pub entries: Vec<(StoredImageSummary, Score, ModelVersion)>,
    pub skeet_appraisals: HashMap<SkeetId, Appraisal>,
    pub image_appraisals: HashMap<ImageId, Appraisal>,
    pub models: Arc<RefineModels>,
}

impl FeedData for CachedFeed {
    fn entries(&self) -> &[(StoredImageSummary, Score, ModelVersion)] {
        &self.entries
    }

    fn image_band(&self, image_id: &ImageId) -> Option<Band> {
        self.image_appraisals.get(image_id).map(|a| a.band)
    }

    fn skeet_band(&self, skeet_id: &SkeetId) -> Option<Band> {
        self.skeet_appraisals.get(skeet_id).map(|a| a.band)
    }

    fn models(&self) -> &RefineModels {
        self.models.as_ref()
    }
}

struct CacheEntry {
    feed: CachedFeed,
    refreshed_at: DateTime<Utc>,
    snapshot: HashSet<Version>,
}

pub struct FeedCache {
    store: Arc<SkeetStore>,
    models: Arc<RefineModels>,
    limit: usize,
    max_age_hours: u64,
    inner: RwLock<Option<CacheEntry>>,
}

/// Outcome of a `refresh_if_changed` call: whether the cache was actually
/// refreshed or skipped because no relevant table version had moved.
#[derive(Debug)]
pub enum RefreshOutcome {
    Unchanged,
    Refreshed { entry_count: usize },
}

impl FeedCache {
    pub fn new(
        store: Arc<SkeetStore>,
        models: Arc<RefineModels>,
        limit: usize,
        max_age_hours: u64,
    ) -> Self {
        Self {
            store,
            models,
            limit,
            max_age_hours,
            inner: RwLock::new(None),
        }
    }

    pub async fn get(&self) -> Result<CachedFeed, skeet_store::StoreError> {
        {
            let guard = self.inner.read().await;
            if let Some(entry) = guard.as_ref() {
                return Ok(entry.feed.clone());
            }
        }
        self.refresh().await
    }

    pub async fn refresh(&self) -> Result<CachedFeed, skeet_store::StoreError> {
        let snapshot = self.store.version_snapshot().await?;
        self.fetch_and_store(snapshot).await
    }

    /// Refresh only if the relevant subset of `version_snapshot` has changed
    /// since the last refresh. Used by the background worker to avoid paying
    /// the full fetch cost when no scoring or appraisal writes have landed.
    pub async fn refresh_if_changed(&self) -> Result<RefreshOutcome, skeet_store::StoreError> {
        let snapshot = self.store.version_snapshot().await?;
        let needs_refresh = {
            let guard = self.inner.read().await;
            guard
                .as_ref()
                .is_none_or(|entry| relevant(&entry.snapshot) != relevant(&snapshot))
        };
        if !needs_refresh {
            return Ok(RefreshOutcome::Unchanged);
        }
        let feed = self.fetch_and_store(snapshot).await?;
        Ok(RefreshOutcome::Refreshed {
            entry_count: feed.entries.len(),
        })
    }

    async fn fetch_and_store(
        &self,
        snapshot: HashSet<Version>,
    ) -> Result<CachedFeed, skeet_store::StoreError> {
        let entries = self
            .store
            .list_scored_summaries_by_score(self.limit, Some(self.max_age_hours))
            .await?;

        let skeet_appraisals: HashMap<SkeetId, Appraisal> = self
            .store
            .list_all_skeet_appraisals()
            .await?
            .into_iter()
            .collect();

        let image_appraisals: HashMap<ImageId, Appraisal> = self
            .store
            .list_all_image_appraisals()
            .await?
            .into_iter()
            .collect();

        info!(
            scored = entries.len(),
            skeet_appraisals = skeet_appraisals.len(),
            image_appraisals = image_appraisals.len(),
            "cache refreshed"
        );

        let feed = CachedFeed {
            entries,
            skeet_appraisals,
            image_appraisals,
            models: self.models.clone(),
        };
        let cloned = feed.clone();
        {
            let mut guard = self.inner.write().await;
            *guard = Some(CacheEntry {
                feed,
                refreshed_at: Utc::now(),
                snapshot,
            });
        }

        Ok(cloned)
    }

    pub async fn refreshed_at(&self) -> Option<DateTime<Utc>> {
        let guard = self.inner.read().await;
        guard.as_ref().map(|entry| entry.refreshed_at)
    }

    pub fn spawn_background_refresh(self: &Arc<Self>) {
        let cache = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(BACKGROUND_REFRESH_INTERVAL).await;
                match cache.refresh_if_changed().await {
                    Ok(RefreshOutcome::Unchanged) => {
                        info!("background cache refresh skipped — no relevant version change");
                    }
                    Ok(RefreshOutcome::Refreshed { entry_count }) => {
                        info!(count = entry_count, "background cache refresh complete");
                    }
                    Err(e) => {
                        warn!(error = %e, "background cache refresh failed");
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Appraiser, Band};
    use skeet_store::ModelVersion;
    use skeet_store::test_utils::{make_record, open_temp_store};
    use test_support::test_models;

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

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        let result = cache.get().await.expect("get");
        assert_eq!(result.entries.len(), 1);
        assert!(cache.refreshed_at().await.is_some());
    }

    #[tokio::test]
    async fn empty_store_returns_empty_cache() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        let result = cache.get().await.expect("get");
        assert!(result.entries.is_empty());
    }

    #[tokio::test]
    async fn refresh_picks_up_new_data() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        let first = cache.get().await.expect("first get");
        assert_eq!(first.entries.len(), 1);

        seed_store(&store, "b", 20, 0.8).await;
        let refreshed = cache.refresh().await.expect("refresh");
        assert_eq!(refreshed.entries.len(), 2);

        let after = cache.get().await.expect("get after refresh");
        assert_eq!(after.entries.len(), 2);
    }

    #[tokio::test]
    async fn refresh_if_changed_populates_cold_cache() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        let outcome = cache.refresh_if_changed().await.expect("refresh");
        assert!(
            matches!(outcome, RefreshOutcome::Refreshed { entry_count: 1 }),
            "cold cache should refresh, got {outcome:?}"
        );

        let result = cache.get().await.expect("get");
        assert_eq!(result.entries.len(), 1);
    }

    #[tokio::test]
    async fn refresh_if_changed_skips_when_no_relevant_change() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        cache.get().await.expect("first get");

        // Add a new image but don't score it — this changes the images table
        // version (irrelevant) but not the scores or appraisals tables.
        let extra = make_record("b", 20, 0, 0);
        store.add(&extra).await.expect("add image");

        let outcome = cache.refresh_if_changed().await.expect("refresh");
        assert!(
            matches!(outcome, RefreshOutcome::Unchanged),
            "irrelevant write should not trigger refresh, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn refresh_if_changed_refreshes_when_score_added() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        cache.get().await.expect("first get");

        seed_store(&store, "b", 20, 0.8).await;

        let outcome = cache.refresh_if_changed().await.expect("refresh");
        assert!(
            matches!(outcome, RefreshOutcome::Refreshed { entry_count: 2 }),
            "score upsert should trigger refresh, got {outcome:?}"
        );

        let after = cache.get().await.expect("get after refresh");
        assert_eq!(after.entries.len(), 2);
    }

    #[tokio::test]
    async fn refresh_if_changed_refreshes_when_appraisal_added() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        cache.get().await.expect("first get");

        let appraiser = Appraiser::new_github("tester").expect("valid appraiser");
        let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/cache_appraise"
            .parse()
            .expect("valid AT URI");
        store
            .set_skeet_band(&skeet_id, Band::HighQuality, &appraiser)
            .await
            .expect("set band");

        let outcome = cache.refresh_if_changed().await.expect("refresh");
        assert!(
            matches!(outcome, RefreshOutcome::Refreshed { .. }),
            "appraisal write should trigger refresh, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn refresh_if_changed_skips_after_consecutive_unchanged_calls() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        cache.get().await.expect("first get");

        for _ in 0..3 {
            let outcome = cache.refresh_if_changed().await.expect("refresh");
            assert!(
                matches!(outcome, RefreshOutcome::Unchanged),
                "expected Unchanged, got {outcome:?}"
            );
        }
    }

    #[tokio::test]
    async fn refreshed_at_returns_some_after_get() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        seed_store(&store, "a", 10, 0.9).await;

        let cache = FeedCache::new(Arc::clone(&store), test_models(), 10, 48);
        assert!(cache.refreshed_at().await.is_none());

        cache.get().await.expect("get");
        let refreshed = cache.refreshed_at().await;
        assert!(refreshed.is_some(), "refreshed_at should be Some after get");
    }
}
