use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use shared::{Rejection, RejectionCategory};
use tracing::info;

use crate::metrics::PruneMetrics;
use crate::pipeline::{
    ChannelMonitors, ContentCounts, PipelineCounters, PipelineSnapshot, PipelineStages,
    RejectionBreakdown, StageStats,
};

const ALL_CATEGORIES: [RejectionCategory; 3] = [
    RejectionCategory::Face,
    RejectionCategory::Text,
    RejectionCategory::Metadata,
];

/// Running rejection tallies: the headline count of rejected images plus the
/// per-reason and per-category breakdowns.
#[derive(Default)]
struct RejectionTally {
    total: u64,
    breakdown: RejectionBreakdown,
}

/// Governs when `log_summary` fires and anchors the cumulative-rate clock.
struct LogCadence {
    started_at: Instant,
    last_log: Instant,
    interval: Duration,
    every_n: u64,
}

pub struct Status {
    content: ContentCounts,
    rejections: RejectionTally,
    cadence: LogCadence,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
    metrics: PruneMetrics,
}

impl Status {
    pub fn new(
        log_interval: Duration,
        log_every_n: u64,
        counters: Arc<PipelineCounters>,
        channels: ChannelMonitors,
    ) -> Self {
        let now = Instant::now();
        Self {
            content: ContentCounts::default(),
            rejections: RejectionTally::default(),
            cadence: LogCadence {
                started_at: now,
                last_log: now,
                interval: log_interval,
                every_n: log_every_n,
            },
            counters,
            channels,
            metrics: PruneMetrics::new(&opentelemetry::global::meter("skeet_prune")),
        }
    }

    pub fn record_post(&mut self, image_count: u64) {
        self.content.posts += 1;
        self.content.images += image_count;
        self.maybe_log();
    }

    pub const fn record_saved(&mut self) {
        self.content.saved += 1;
    }

    pub fn record_rejected(&mut self, reasons: &[Rejection]) {
        self.rejections.total += 1;
        let breakdown = &mut self.rejections.breakdown;
        let mut categories_seen: HashSet<RejectionCategory> = HashSet::new();
        for reason in reasons {
            *breakdown.by_reason.entry(*reason).or_default() += 1;
            categories_seen.insert(reason.category());
        }
        for &cat in &categories_seen {
            *breakdown.by_category.entry(cat).or_default() += 1;
        }
        if categories_seen.len() == 1
            && let Some(sole) = categories_seen.into_iter().next()
        {
            *breakdown.by_sole_category.entry(sole).or_default() += 1;
        }
    }

    pub const fn saved_count(&self) -> u64 {
        self.content.saved
    }

    fn maybe_log(&mut self) {
        if self.content.posts == 1
            || self.content.posts.is_multiple_of(self.cadence.every_n)
            || self.cadence.last_log.elapsed() >= self.cadence.interval
        {
            self.log_summary();
            self.cadence.last_log = Instant::now();
        }
    }

    // Every `expect` below is a `write!` into a `String`, which is infallible.
    #[allow(clippy::expect_used)]
    fn log_summary(&mut self) {
        let hit_rate = if self.content.images > 0 {
            (self.content.saved as f64 / self.content.images as f64) * 100.0
        } else {
            0.0
        };

        let posts = self.content.posts;
        let images = self.content.images;
        let saved = self.content.saved;
        let rejected = self.rejections.total;

        let elapsed = self.cadence.started_at.elapsed().as_secs_f64();
        let skeets_per_sec = if elapsed > 0.0 {
            posts as f64 / elapsed
        } else {
            0.0
        };

        let saved_detail = format!("saved: {saved} ({hit_rate:.1}%)");

        let mut msg = format!(
            "skeets: {posts} ({skeets_per_sec:.1}/s) | images: {images} | {saved_detail} | rejected: {rejected}"
        );

        if !self.rejections.breakdown.by_reason.is_empty() {
            let total_reasons: u64 = self.rejections.breakdown.by_reason.values().sum();
            let mut sorted: Vec<_> = self.rejections.breakdown.by_reason.iter().collect();
            sorted.sort_by_key(|(r, _)| r.to_string());

            write!(msg, " (").expect("write to String");
            for (i, (reason, count)) in sorted.iter().enumerate() {
                let pct = (**count as f64 / total_reasons as f64) * 100.0;
                if i > 0 {
                    write!(msg, ", ").expect("write to String");
                }
                write!(msg, "{reason}: {count} [{pct:.0}%]").expect("write to String");
            }
            write!(msg, ")").expect("write to String");
        }

        if !self.rejections.breakdown.by_category.is_empty() {
            write!(msg, " | categories: ").expect("write to String");
            for (i, cat) in ALL_CATEGORIES.iter().enumerate() {
                let count = self
                    .rejections
                    .breakdown
                    .by_category
                    .get(cat)
                    .copied()
                    .unwrap_or(0);
                let pct = if rejected > 0 {
                    (count as f64 / rejected as f64) * 100.0
                } else {
                    0.0
                };
                let sole = self
                    .rejections
                    .breakdown
                    .by_sole_category
                    .get(cat)
                    .copied()
                    .unwrap_or(0);
                let sole_pct = if rejected > 0 {
                    (sole as f64 / rejected as f64) * 100.0
                } else {
                    0.0
                };
                if i > 0 {
                    write!(msg, ", ").expect("write to String");
                }
                write!(
                    msg,
                    "{cat}: {count} [{pct:.0}%] (sole: {sole} [{sole_pct:.0}%])"
                )
                .expect("write to String");
            }
        }

        info!("{msg}");

        let firehose = self.counters.firehose_count();
        let meta = self.counters.meta_count();
        let image = self.counters.image_count();

        let firehose_per_sec = if elapsed > 0.0 {
            firehose as f64 / elapsed
        } else {
            0.0
        };
        let meta_per_sec = if elapsed > 0.0 {
            meta as f64 / elapsed
        } else {
            0.0
        };
        let image_per_sec = if elapsed > 0.0 {
            image as f64 / elapsed
        } else {
            0.0
        };

        let firehose_depth = self.channels.firehose_depth();
        let meta_depth = self.channels.meta_depth();
        let image_depth = self.channels.image_depth();

        info!(
            "pipeline | throughput: firehose={firehose} ({firehose_per_sec:.1}/s), \
             meta={meta} ({meta_per_sec:.1}/s), image={image} ({image_per_sec:.1}/s) \
             | depth: firehose={firehose_depth}, meta={meta_depth}, image={image_depth}",
        );

        let snapshot = PipelineSnapshot {
            stages: PipelineStages {
                firehose: StageStats {
                    throughput: firehose,
                    depth: firehose_depth,
                },
                meta: StageStats {
                    throughput: meta,
                    depth: meta_depth,
                },
                image: StageStats {
                    throughput: image,
                    depth: image_depth,
                },
            },
            content: self.content.clone(),
            rejections: self.rejections.breakdown.clone(),
        };
        self.metrics.emit(&snapshot);
    }
}
