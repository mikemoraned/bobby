use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use skeet_store::{PruneStats, Statistics};
use tracing::error;

use crate::pipeline::ContentCounts;
use crate::pipeline::content_counts_recorder::ContentCountsRecorder;

/// Records one [`PruneStats`] to the store per `store_interval`, carrying the
/// counts accumulated since the previous record and the wall-clock bounds of
/// that slice.
pub struct StatisticsPersister<'a> {
    statistics: &'a dyn Statistics,
    store_interval: Duration,
    since_flush: ContentCounts,
    interval_start: DateTime<Utc>,
    last_flush: Instant,
}

impl<'a> StatisticsPersister<'a> {
    pub fn new(statistics: &'a dyn Statistics, store_interval: Duration) -> Self {
        Self {
            statistics,
            store_interval,
            since_flush: ContentCounts::default(),
            interval_start: Utc::now(),
            last_flush: Instant::now(),
        }
    }
}

#[async_trait]
impl ContentCountsRecorder for StatisticsPersister<'_> {
    async fn record_counts(&mut self, counts: &ContentCounts) {
        self.since_flush += counts;
        if self.last_flush.elapsed() < self.store_interval {
            return;
        }

        let now = Utc::now();
        let stats = PruneStats {
            interval_start: self.interval_start,
            interval_end: now,
            skeets_seen: self.since_flush.posts,
            images_examined: self.since_flush.images,
            images_saved: self.since_flush.saved,
        };
        match self.statistics.record_prune_stats(&stats).await {
            Ok(()) => {
                self.since_flush = ContentCounts::default();
                self.interval_start = now;
                self.last_flush = Instant::now();
            }
            Err(e) => error!(error = %e, "failed to record prune statistics; will retry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::Utc;
    use skeet_store::StoreError;
    use skeet_store::test_utils::open_temp_store;

    use super::*;

    /// A `Statistics` whose `record` fails the first time, then succeeds —
    /// capturing the stats it eventually persists.
    #[derive(Default)]
    struct FlakyStatistics {
        fail_next: Mutex<bool>,
        recorded: Mutex<Vec<PruneStats>>,
    }

    #[async_trait]
    impl Statistics for FlakyStatistics {
        async fn record_prune_stats(&self, stats: &PruneStats) -> Result<(), StoreError> {
            let mut fail_next = self.fail_next.lock().unwrap();
            if *fail_next {
                *fail_next = false;
                return Err(StoreError::ValidationFailed("boom".into()));
            }
            self.recorded.lock().unwrap().push(stats.clone());
            Ok(())
        }

        async fn prune_stats_for_interval(
            &self,
            _start: DateTime<Utc>,
            _end: DateTime<Utc>,
        ) -> Result<PruneStats, StoreError> {
            unreachable!("not used in these tests")
        }

        async fn latest_prune_stats_interval_end(
            &self,
        ) -> Result<Option<DateTime<Utc>>, StoreError> {
            unreachable!("not used in these tests")
        }

        async fn count_images(&self) -> Result<usize, StoreError> {
            unreachable!("not used in these tests")
        }

        async fn count_images_in_interval(
            &self,
            _start: DateTime<Utc>,
            _end: DateTime<Utc>,
        ) -> Result<u64, StoreError> {
            unreachable!("not used in these tests")
        }

        async fn count_scores_by_model_version(
            &self,
        ) -> Result<std::collections::HashMap<String, usize>, StoreError> {
            unreachable!("not used in these tests")
        }
    }

    #[tokio::test]
    async fn keeps_accumulating_when_record_fails() {
        let stats = FlakyStatistics {
            fail_next: Mutex::new(true),
            recorded: Mutex::new(Vec::new()),
        };
        let mut persister = StatisticsPersister::new(&stats, Duration::ZERO);

        // First flush fails: its counts must be retained, not dropped.
        persister
            .record_counts(&(ContentCounts::post(3) + ContentCounts::saved()))
            .await;
        // Next flush succeeds: it must carry both messages' counts.
        persister.record_counts(&ContentCounts::post(1)).await;

        let recorded = stats.recorded.lock().unwrap();
        assert_eq!(recorded.len(), 1, "only the successful flush is recorded");
        assert_eq!(recorded[0].skeets_seen, 2);
        assert_eq!(recorded[0].images_examined, 4);
        assert_eq!(recorded[0].images_saved, 1);
    }

    #[tokio::test]
    async fn records_accumulated_counts_once_the_interval_elapses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;
        let before = Utc::now();

        // A zero interval flushes on every message.
        let mut persister = StatisticsPersister::new(&store, Duration::ZERO);
        persister
            .record_counts(&(ContentCounts::post(3) + ContentCounts::saved()))
            .await;
        persister.record_counts(&ContentCounts::post(1)).await;

        let recorded = store
            .prune_stats_for_interval(before, Utc::now())
            .await
            .expect("interval counts");
        assert_eq!(recorded.skeets_seen, 2);
        assert_eq!(recorded.images_examined, 4);
        assert_eq!(recorded.images_saved, 1);
    }

    #[tokio::test]
    async fn accumulates_without_recording_until_the_interval_elapses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;
        let before = Utc::now();

        // A long interval means nothing is recorded yet.
        let mut persister = StatisticsPersister::new(&store, Duration::from_secs(3600));
        persister.record_counts(&ContentCounts::post(5)).await;

        let recorded = store
            .prune_stats_for_interval(before, Utc::now())
            .await
            .expect("interval counts");
        assert_eq!(recorded.skeets_seen, 0);
        assert_eq!(recorded.images_examined, 0);
    }
}
