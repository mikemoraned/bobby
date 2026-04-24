use std::collections::HashMap;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge},
};
use shared::{Rejection, RejectionCategory};

/// OTel metrics emitted by skeet-prune at each status log interval.
pub struct PruneMetrics {
    throughput: Counter<u64>,
    depth: Gauge<u64>,
    skeets_total: Counter<u64>,
    images_total: Counter<u64>,
    saved_total: Counter<u64>,
    rejected_total: Counter<u64>,
    categories_total: Counter<u64>,
    categories_sole_total: Counter<u64>,

    // Tracks the cumulative totals emitted so far; deltas are added each interval.
    prev_firehose: u64,
    prev_meta: u64,
    prev_image: u64,
    prev_skeets: u64,
    prev_images: u64,
    prev_saved: u64,
    prev_rejected: HashMap<Rejection, u64>,
    prev_categories: HashMap<RejectionCategory, u64>,
    prev_sole_categories: HashMap<RejectionCategory, u64>,
}

impl PruneMetrics {
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("skeet_prune");

        Self {
            throughput: meter
                .u64_counter("skeet_prune.pipeline.throughput")
                .with_description("Cumulative items processed per pipeline stage")
                .with_unit("items")
                .build(),
            depth: meter
                .u64_gauge("skeet_prune.pipeline.depth")
                .with_description("Current queue depth per pipeline stage")
                .with_unit("items")
                .build(),
            skeets_total: meter
                .u64_counter("skeet_prune.skeets.total")
                .with_description("Cumulative skeets seen")
                .with_unit("skeets")
                .build(),
            images_total: meter
                .u64_counter("skeet_prune.images.total")
                .with_description("Cumulative images seen")
                .with_unit("images")
                .build(),
            saved_total: meter
                .u64_counter("skeet_prune.saved.total")
                .with_description("Cumulative images saved")
                .with_unit("images")
                .build(),
            rejected_total: meter
                .u64_counter("skeet_prune.rejected.total")
                .with_description("Cumulative images rejected, by rejection reason")
                .with_unit("images")
                .build(),
            categories_total: meter
                .u64_counter("skeet_prune.categories.total")
                .with_description("Cumulative rejected images by detection category")
                .with_unit("images")
                .build(),
            categories_sole_total: meter
                .u64_counter("skeet_prune.categories.sole.total")
                .with_description(
                    "Cumulative rejected images where one category was the sole detection",
                )
                .with_unit("images")
                .build(),
            prev_firehose: 0,
            prev_meta: 0,
            prev_image: 0,
            prev_skeets: 0,
            prev_images: 0,
            prev_saved: 0,
            prev_rejected: HashMap::new(),
            prev_categories: HashMap::new(),
            prev_sole_categories: HashMap::new(),
        }
    }

    /// Emit metrics for one status interval. Counters receive the delta since the
    /// last call; gauges receive the current value.
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &mut self,
        firehose_count: u64,
        meta_count: u64,
        image_count: u64,
        firehose_depth: usize,
        meta_depth: usize,
        image_depth: usize,
        skeets: u64,
        images: u64,
        saved: u64,
        rejection_counts: &HashMap<Rejection, u64>,
        category_counts: &HashMap<RejectionCategory, u64>,
        sole_category_counts: &HashMap<RejectionCategory, u64>,
    ) {
        // Pipeline throughput — emit delta per stage
        let firehose_delta = firehose_count.saturating_sub(self.prev_firehose);
        if firehose_delta > 0 {
            self.throughput
                .add(firehose_delta, &[KeyValue::new("stage", "firehose")]);
            self.prev_firehose = firehose_count;
        }
        let meta_delta = meta_count.saturating_sub(self.prev_meta);
        if meta_delta > 0 {
            self.throughput
                .add(meta_delta, &[KeyValue::new("stage", "meta")]);
            self.prev_meta = meta_count;
        }
        let image_delta = image_count.saturating_sub(self.prev_image);
        if image_delta > 0 {
            self.throughput
                .add(image_delta, &[KeyValue::new("stage", "image")]);
            self.prev_image = image_count;
        }

        // Pipeline depth — emit current value
        self.depth
            .record(firehose_depth as u64, &[KeyValue::new("stage", "firehose")]);
        self.depth
            .record(meta_depth as u64, &[KeyValue::new("stage", "meta")]);
        self.depth
            .record(image_depth as u64, &[KeyValue::new("stage", "image")]);

        // Content counters — emit delta
        let skeets_delta = skeets.saturating_sub(self.prev_skeets);
        if skeets_delta > 0 {
            self.skeets_total.add(skeets_delta, &[]);
            self.prev_skeets = skeets;
        }

        let images_delta = images.saturating_sub(self.prev_images);
        if images_delta > 0 {
            self.images_total.add(images_delta, &[]);
            self.prev_images = images;
        }

        let saved_delta = saved.saturating_sub(self.prev_saved);
        if saved_delta > 0 {
            self.saved_total.add(saved_delta, &[]);
            self.prev_saved = saved;
        }

        for (reason, &count) in rejection_counts {
            let prev = self.prev_rejected.get(reason).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            if delta > 0 {
                self.rejected_total
                    .add(delta, &[KeyValue::new("reason", reason.to_string())]);
                self.prev_rejected.insert(*reason, count);
            }
        }

        for (cat, &count) in category_counts {
            let prev = self.prev_categories.get(cat).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            if delta > 0 {
                self.categories_total
                    .add(delta, &[KeyValue::new("category", cat.to_string())]);
                self.prev_categories.insert(*cat, count);
            }
        }

        for (cat, &count) in sole_category_counts {
            let prev = self.prev_sole_categories.get(cat).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            if delta > 0 {
                self.categories_sole_total
                    .add(delta, &[KeyValue::new("category", cat.to_string())]);
                self.prev_sole_categories.insert(*cat, count);
            }
        }
    }
}
