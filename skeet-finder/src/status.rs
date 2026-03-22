use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use shared::Rejection;
use tracing::info;

pub struct Status {
    post_count: u64,
    image_count: u64,
    saved_count: u64,
    rejected_count: u64,
    rejection_counts: HashMap<Rejection, u64>,
    last_log: Instant,
    log_interval: Duration,
    log_every_n: u64,
    started_at: Instant,
}

impl Status {
    pub fn new(log_interval: Duration, log_every_n: u64) -> Self {
        Self {
            post_count: 0,
            image_count: 0,
            saved_count: 0,
            rejected_count: 0,
            rejection_counts: HashMap::new(),
            last_log: Instant::now(),
            log_interval,
            log_every_n,
            started_at: Instant::now(),
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

    pub fn record_rejected(&mut self, reasons: &[Rejection]) {
        self.rejected_count += 1;
        for reason in reasons {
            *self.rejection_counts.entry(*reason).or_default() += 1;
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

        let mut msg = format!(
            "skeets: {posts} ({skeets_per_sec:.1}/s) | images: {images} | saved: {saved} ({hit_rate:.1}%) | rejected: {rejected}"
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

        info!("{msg}");
    }
}
