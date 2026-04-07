use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use shared::{Rejection, RejectionCategory};
use tracing::info;

use crate::pipeline::{ChannelMonitors, PipelineCounters};

const ALL_CATEGORIES: [RejectionCategory; 2] = [
    RejectionCategory::Face,
    RejectionCategory::Metadata,
];

pub struct Status {
    post_count: u64,
    image_count: u64,
    saved_count: u64,
    saved_remote: u64,
    saved_fallback: u64,
    rejected_count: u64,
    rejection_counts: HashMap<Rejection, u64>,
    category_counts: HashMap<RejectionCategory, u64>,
    sole_category_counts: HashMap<RejectionCategory, u64>,
    last_log: Instant,
    log_interval: Duration,
    log_every_n: u64,
    started_at: Instant,
    counters: Arc<PipelineCounters>,
    channels: ChannelMonitors,
}

impl Status {
    pub fn new(
        log_interval: Duration,
        log_every_n: u64,
        counters: Arc<PipelineCounters>,
        channels: ChannelMonitors,
    ) -> Self {
        Self {
            post_count: 0,
            image_count: 0,
            saved_count: 0,
            saved_remote: 0,
            saved_fallback: 0,
            rejected_count: 0,
            rejection_counts: HashMap::new(),
            category_counts: HashMap::new(),
            sole_category_counts: HashMap::new(),
            last_log: Instant::now(),
            log_interval,
            log_every_n,
            started_at: Instant::now(),
            counters,
            channels,
        }
    }

    pub fn record_post(&mut self, image_count: u64) {
        self.post_count += 1;
        self.image_count += image_count;
        self.maybe_log();
    }

    pub const fn record_saved(&mut self) {
        self.saved_count += 1;
    }

    pub const fn record_saved_remote(&mut self) {
        self.saved_count += 1;
        self.saved_remote += 1;
    }

    pub const fn record_saved_fallback(&mut self) {
        self.saved_count += 1;
        self.saved_fallback += 1;
    }

    pub fn record_rejected(&mut self, reasons: &[Rejection]) {
        self.rejected_count += 1;
        let mut categories_seen: HashSet<RejectionCategory> = HashSet::new();
        for reason in reasons {
            *self.rejection_counts.entry(*reason).or_default() += 1;
            categories_seen.insert(reason.category());
        }
        for &cat in &categories_seen {
            *self.category_counts.entry(cat).or_default() += 1;
        }
        if categories_seen.len() == 1 {
            let sole = categories_seen.into_iter().next().expect("just checked len == 1");
            *self.sole_category_counts.entry(sole).or_default() += 1;
        }
    }

    pub const fn saved_count(&self) -> u64 {
        self.saved_count
    }

    fn maybe_log(&mut self) {
        if self.post_count == 1
            || self.post_count.is_multiple_of(self.log_every_n)
            || self.last_log.elapsed() >= self.log_interval
        {
            self.log_summary();
            self.last_log = Instant::now();
        }
    }

    fn log_summary(&self) {
        let hit_rate = if self.image_count > 0 {
            (self.saved_count as f64 / self.image_count as f64) * 100.0
        } else {
            0.0
        };

        let posts = self.post_count;
        let images = self.image_count;
        let saved = self.saved_count;
        let rejected = self.rejected_count;

        let elapsed = self.started_at.elapsed().as_secs_f64();
        let skeets_per_sec = if elapsed > 0.0 {
            posts as f64 / elapsed
        } else {
            0.0
        };

        let saved_detail = if self.saved_fallback > 0 {
            format!(
                "saved: {saved} ({hit_rate:.1}%) [remote: {}, fallback: {}]",
                self.saved_remote, self.saved_fallback
            )
        } else {
            format!("saved: {saved} ({hit_rate:.1}%)")
        };

        let mut msg = format!(
            "skeets: {posts} ({skeets_per_sec:.1}/s) | images: {images} | {saved_detail} | rejected: {rejected}"
        );

        if !self.rejection_counts.is_empty() {
            let total_reasons: u64 = self.rejection_counts.values().sum();
            let mut sorted: Vec<_> = self.rejection_counts.iter().collect();
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

        if !self.category_counts.is_empty() {
            write!(msg, " | categories: ").expect("write to String");
            for (i, cat) in ALL_CATEGORIES.iter().enumerate() {
                let count = self.category_counts.get(cat).copied().unwrap_or(0);
                let pct = if rejected > 0 {
                    (count as f64 / rejected as f64) * 100.0
                } else {
                    0.0
                };
                let sole = self.sole_category_counts.get(cat).copied().unwrap_or(0);
                let sole_pct = if rejected > 0 {
                    (sole as f64 / rejected as f64) * 100.0
                } else {
                    0.0
                };
                if i > 0 {
                    write!(msg, ", ").expect("write to String");
                }
                write!(msg, "{cat}: {count} [{pct:.0}%] (sole: {sole} [{sole_pct:.0}%])").expect("write to String");
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

        info!(
            "pipeline | throughput: firehose={firehose} ({firehose_per_sec:.1}/s), \
             meta={meta} ({meta_per_sec:.1}/s), image={image} ({image_per_sec:.1}/s) \
             | depth: firehose={}, meta={}, image={}",
            self.channels.firehose_depth(),
            self.channels.meta_depth(),
            self.channels.image_depth(),
        );
    }
}
