use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use skeet_store::{PruneStats, Statistics};
use tracing::error;

use crate::pipeline::ContentCounts;
use crate::pipeline::content_counts_recorder::ContentCountsRecorder;

/// Buffers per-interval [`PruneStats`] and writes them to the store in batches.
///
/// Counts are aggregated into one [`PruneStats`] per `record_interval` (each
/// carrying the counts since the previous record and its wall-clock bounds), but
/// the records are buffered and written together once per `flush_interval` — a
/// single multi-row append instead of one append per record, which amortises the
/// store's fragment/manifest churn while keeping per-interval granularity in the
/// stored rows.
pub struct StatisticsPersister<'a> {
    statistics: &'a dyn Statistics,
    record_interval: Duration,
    flush_interval: Duration,
    since_record: ContentCounts,
    interval_start: DateTime<Utc>,
    last_record: Instant,
    buffer: Vec<PruneStats>,
    last_flush: Instant,
}

impl<'a> StatisticsPersister<'a> {
    pub fn new(
        statistics: &'a dyn Statistics,
        record_interval: Duration,
        flush_interval: Duration,
    ) -> Self {
        let now = Instant::now();
        Self {
            statistics,
            record_interval,
            flush_interval,
            since_record: ContentCounts::default(),
            interval_start: Utc::now(),
            last_record: now,
            buffer: Vec::new(),
            last_flush: now,
        }
    }
}

#[async_trait]
impl ContentCountsRecorder for StatisticsPersister<'_> {
    async fn record_counts(&mut self, counts: &ContentCounts) {
        self.since_record += counts;
        if self.last_record.elapsed() >= self.record_interval {
            let now = Utc::now();
            self.buffer.push(PruneStats {
                interval_start: self.interval_start,
                interval_end: now,
                skeets_seen: self.since_record.posts,
                images_examined: self.since_record.images,
                images_saved: self.since_record.saved,
            });
            self.since_record = ContentCounts::default();
            self.interval_start = now;
            self.last_record = Instant::now();
        }

        if self.last_flush.elapsed() >= self.flush_interval {
            self.flush().await;
        }
    }

    /// Write all buffered records in one append. On failure the buffer is
    /// retained so the next flush retries; on success it is cleared.
    async fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        match self.statistics.record_prune_stats(&self.buffer).await {
            Ok(()) => {
                self.buffer.clear();
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
    /// capturing each batch it eventually persists.
    #[derive(Default)]
    struct FlakyStatistics {
        fail_next: Mutex<bool>,
        flushes: Mutex<Vec<Vec<PruneStats>>>,
    }

    #[async_trait]
    impl Statistics for FlakyStatistics {
        async fn record_prune_stats(&self, stats: &[PruneStats]) -> Result<(), StoreError> {
            let mut fail_next = self.fail_next.lock().unwrap();
            if *fail_next {
                *fail_next = false;
                return Err(StoreError::ValidationFailed("boom".into()));
            }
            self.flushes.lock().unwrap().push(stats.to_vec());
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
    async fn keeps_buffering_when_a_flush_fails() {
        let stats = FlakyStatistics {
            fail_next: Mutex::new(true),
            flushes: Mutex::new(Vec::new()),
        };
        // Record and flush on every message.
        let mut persister = StatisticsPersister::new(&stats, Duration::ZERO, Duration::ZERO);

        // First flush fails: its record must be retained in the buffer, not dropped.
        persister
            .record_counts(&(ContentCounts::post(3) + ContentCounts::saved()))
            .await;
        // Next flush succeeds: it must carry both records, the retained and the new.
        persister.record_counts(&ContentCounts::post(1)).await;

        let flushes = stats.flushes.lock().unwrap();
        assert_eq!(flushes.len(), 1, "only the successful flush is recorded");
        let batch = &flushes[0];
        assert_eq!(batch.len(), 2, "the retained record rides the next batch");
        assert_eq!(batch.iter().map(|s| s.skeets_seen).sum::<u64>(), 2);
        assert_eq!(batch.iter().map(|s| s.images_examined).sum::<u64>(), 4);
        assert_eq!(batch.iter().map(|s| s.images_saved).sum::<u64>(), 1);
    }

    #[tokio::test]
    async fn records_accumulated_counts_once_the_interval_elapses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;
        let before = Utc::now();

        // Zero intervals record and flush on every message.
        let mut persister = StatisticsPersister::new(&store, Duration::ZERO, Duration::ZERO);
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

        // A long record interval means no PruneStats is produced yet.
        let mut persister =
            StatisticsPersister::new(&store, Duration::from_secs(3600), Duration::ZERO);
        persister.record_counts(&ContentCounts::post(5)).await;

        let recorded = store
            .prune_stats_for_interval(before, Utc::now())
            .await
            .expect("interval counts");
        assert_eq!(recorded.skeets_seen, 0);
        assert_eq!(recorded.images_examined, 0);
    }

    #[tokio::test]
    async fn buffers_records_until_flushed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = open_temp_store(&dir).await;
        let before = Utc::now();

        // Record on every message, but only flush on shutdown (long flush interval).
        let mut persister =
            StatisticsPersister::new(&store, Duration::ZERO, Duration::from_secs(3600));
        persister.record_counts(&ContentCounts::post(3)).await;
        persister.record_counts(&ContentCounts::post(2)).await;

        // Nothing written to the store before the flush.
        let pre_flush = store
            .prune_stats_for_interval(before, Utc::now())
            .await
            .expect("interval counts");
        assert_eq!(pre_flush.images_examined, 0);

        persister.flush().await;

        // The flush writes both buffered records as one batch.
        let recorded = store
            .prune_stats_for_interval(before, Utc::now())
            .await
            .expect("interval counts");
        assert_eq!(recorded.skeets_seen, 2);
        assert_eq!(recorded.images_examined, 5);
    }
}
