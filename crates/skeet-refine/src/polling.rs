use std::sync::Arc;

use shared::DiscoveredAt;
use skeet_store::{
    Images, ScoredView, StoreError, TableName, TableVersions, Version, VersionedCache,
};
use tracing::info;

use crate::batch::Batch;

pub struct PollingBatchSource<S> {
    store: Arc<S>,
    images_gate: VersionedCache<Version, ()>,
    last_discovered_at: Option<DiscoveredAt>,
}

impl<S: Images + ScoredView + TableVersions> PollingBatchSource<S> {
    pub const fn new(store: Arc<S>) -> Self {
        Self {
            store,
            images_gate: VersionedCache::new(),
            last_discovered_at: None,
        }
    }

    /// Fetch unscored images for this tick.
    ///
    /// "Unscored" means *no row* in the scores table — re-scoring under a
    /// different `model_version` is a deliberate offline operation, not an
    /// automatic side-effect of deploying a new production model.
    ///
    /// Returns an empty `Batch` if the `images` table version hasn't changed since
    /// the last call — skipping the expensive full-table scan. On cold start
    /// (first call) always runs the scan regardless.
    ///
    /// Once a watermark exists (set by [`Self::commit`] after a prior tick), the
    /// underlying scan pushes down a `discovered_at >= last_discovered_at`
    /// filter so the oldest unscored straggler is always re-included.
    pub async fn fetch(&mut self) -> Result<Batch, StoreError> {
        let images_version = self.store.table_version(TableName::Images.as_str()).await?;

        if self.images_gate.is_cached_current(&images_version) {
            return Ok(Batch::default());
        }

        let unscored_ids = self
            .store
            .list_unscored_image_ids(self.last_discovered_at.as_ref())
            .await?;

        self.images_gate.cache(images_version, ());

        if unscored_ids.is_empty() {
            return Ok(Batch::default());
        }

        info!(count = unscored_ids.len(), "found unscored images");

        let originals = self.store.get_originals_by_ids(&unscored_ids).await?;
        Ok(Batch::from(originals.into_values().collect::<Vec<_>>()))
    }

    /// Consume `batch` and advance the internal watermark using its completion
    /// bookkeeping. Monotonic — earlier watermarks never roll the cutoff back.
    pub fn commit(&mut self, batch: Batch) {
        let Some(w) = batch.watermark() else {
            return;
        };
        match &self.last_discovered_at {
            Some(prev) if prev >= &w => {}
            _ => self.last_discovered_at = Some(w),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use skeet_store::test_utils::{make_record, make_record_at, open_temp_store};

    #[tokio::test]
    async fn cold_start_always_fetches() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        store
            .add(&make_record("cold1", 10, 0, 0))
            .await
            .expect("add");

        let mut source = PollingBatchSource::new(store);
        let batch = source.fetch().await.expect("fetch");
        assert_eq!(batch.len(), 1);
    }

    #[tokio::test]
    async fn image_scored_under_any_model_version_is_not_refetched() {
        use skeet_store::{ModelVersion, Score, Scores};

        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        let r = make_record("old_score", 10, 0, 0);
        store.add(&r).await.expect("add");
        store
            .upsert_score(
                &r.image_id,
                &Score::new(0.5).expect("valid"),
                &ModelVersion::from("previous_prompt"),
            )
            .await
            .expect("upsert prior score");

        let mut source = PollingBatchSource::new(store);
        let batch = source.fetch().await.expect("fetch");
        assert!(
            batch.is_empty(),
            "an image already scored under any model_version must not be re-fetched"
        );
    }

    #[tokio::test]
    async fn unchanged_version_returns_empty_batch() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        store
            .add(&make_record("same1", 10, 0, 0))
            .await
            .expect("add");

        let mut source = PollingBatchSource::new(store);
        let _ = source.fetch().await.expect("first fetch");
        let batch = source.fetch().await.expect("second fetch");
        assert!(
            batch.is_empty(),
            "expected early-abort on unchanged version"
        );
    }

    #[tokio::test]
    async fn changed_version_fetches_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        store
            .add(&make_record("chg1", 10, 0, 0))
            .await
            .expect("add");

        let mut source = PollingBatchSource::new(store.clone());
        let first = source.fetch().await.expect("first fetch");
        assert_eq!(first.len(), 1);

        store
            .add(&make_record("chg2", 20, 0, 0))
            .await
            .expect("add");

        let second = source.fetch().await.expect("second fetch");
        assert_eq!(second.len(), 2, "both images should be unscored");
    }

    #[tokio::test]
    async fn batch_owns_discovered_at_per_candidate() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        let t = chrono::Utc.with_ymd_and_hms(2026, 4, 20, 0, 0, 0).unwrap();
        let r = make_record_at("d1", 10, 0, 0, DiscoveredAt::new(t));
        let id = r.image_id.clone();
        store.add(&r).await.expect("add");

        let mut source = PollingBatchSource::new(store);
        let batch = source.fetch().await.expect("fetch");
        assert_eq!(batch.discovered_at(&id), Some(&DiscoveredAt::new(t)));
    }

    #[tokio::test]
    async fn commit_fully_completed_batch_advances_to_max() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        let t_old = chrono::Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        let t_new = chrono::Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap();
        let r_old = make_record_at("ok_old", 10, 0, 0, DiscoveredAt::new(t_old));
        let r_new = make_record_at("ok_new", 20, 0, 0, DiscoveredAt::new(t_new));
        store.add(&r_old).await.expect("add");
        store.add(&r_new).await.expect("add");

        let mut source = PollingBatchSource::new(store.clone());
        let mut first = source.fetch().await.expect("first fetch");
        assert_eq!(first.len(), 2);
        first.mark_completed(&r_old.image_id);
        first.mark_completed(&r_new.image_id);
        source.commit(first);

        assert_eq!(
            source.last_discovered_at.as_ref(),
            Some(&DiscoveredAt::new(t_new)),
            "fully-completed → watermark = max of batch"
        );
    }

    #[tokio::test]
    async fn commit_with_straggler_advances_to_oldest_uncompleted() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        let t_old = chrono::Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        let t_mid = chrono::Utc.with_ymd_and_hms(2026, 4, 15, 0, 0, 0).unwrap();
        let t_new = chrono::Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap();
        let r_old = make_record_at("s_old", 10, 0, 0, DiscoveredAt::new(t_old));
        let r_mid = make_record_at("s_mid", 20, 0, 0, DiscoveredAt::new(t_mid));
        let r_new = make_record_at("s_new", 30, 0, 0, DiscoveredAt::new(t_new));
        store.add(&r_old).await.expect("add");
        store.add(&r_mid).await.expect("add");
        store.add(&r_new).await.expect("add");

        let mut source = PollingBatchSource::new(store.clone());
        let mut first = source.fetch().await.expect("first fetch");
        assert_eq!(first.len(), 3);
        // Old + new succeed; mid fails (e.g. ParseScore error).
        first.mark_completed(&r_old.image_id);
        first.mark_completed(&r_new.image_id);
        source.commit(first);

        assert_eq!(
            source.last_discovered_at.as_ref(),
            Some(&DiscoveredAt::new(t_mid)),
            "watermark = min(uncompleted) so the straggler re-appears"
        );
    }

    #[tokio::test]
    async fn straggler_reappears_on_next_fetch_after_commit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        let t_mid = chrono::Utc.with_ymd_and_hms(2026, 4, 15, 0, 0, 0).unwrap();
        let t_new = chrono::Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap();
        let r_mid = make_record_at("re_mid", 20, 0, 0, DiscoveredAt::new(t_mid));
        let r_new = make_record_at("re_new", 30, 0, 0, DiscoveredAt::new(t_new));
        store.add(&r_mid).await.expect("add");
        store.add(&r_new).await.expect("add");

        let mut source = PollingBatchSource::new(store.clone());
        let mut first = source.fetch().await.expect("first fetch");
        assert_eq!(first.len(), 2);
        first.mark_completed(&r_new.image_id); // mid stays uncompleted
        source.commit(first);

        // Add a row that bumps the table version so the early-abort doesn't fire.
        let t_extra = chrono::Utc.with_ymd_and_hms(2026, 4, 28, 0, 0, 0).unwrap();
        let r_extra = make_record_at("re_extra", 40, 0, 0, DiscoveredAt::new(t_extra));
        store.add(&r_extra).await.expect("add");

        let second = source.fetch().await.expect("second fetch");
        // mid is still unscored; extra is brand new; new is already scored. But
        // we never actually called batch_upsert_scores, so list_unscored sees
        // nothing as scored — so mid + extra + new (still ≥ t_mid) all return.
        // What we're really asserting: mid is in the next batch, i.e. straggler
        // re-appears once the version-snapshot early-abort releases.
        assert!(
            second.discovered_at(&r_mid.image_id).is_some(),
            "the uncompleted straggler must re-appear"
        );
    }

    #[tokio::test]
    async fn commit_is_monotonic_with_constructed_batches() {
        // Watermark is computed from the batch only — exercise commit
        // without going through a store fetch by constructing batches by hand.
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        let mut source = PollingBatchSource::new(store);

        let t_late = chrono::Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap();
        let t_early = chrono::Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        let id_late = make_record_at("late", 10, 0, 0, DiscoveredAt::new(t_late)).image_id;
        let id_early = make_record_at("early", 20, 0, 0, DiscoveredAt::new(t_early)).image_id;

        let mut b1 = Batch::with_entry(id_late.clone(), DiscoveredAt::new(t_late));
        b1.mark_completed(&id_late);
        source.commit(b1);
        assert_eq!(
            source.last_discovered_at.as_ref(),
            Some(&DiscoveredAt::new(t_late))
        );

        // Don't mark — watermark would be t_early.
        let b2 = Batch::with_entry(id_early, DiscoveredAt::new(t_early));
        source.commit(b2);
        assert_eq!(
            source.last_discovered_at.as_ref(),
            Some(&DiscoveredAt::new(t_late)),
            "earlier watermark must not roll the cutoff back"
        );
    }
}
