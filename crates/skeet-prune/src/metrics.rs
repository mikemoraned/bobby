use std::collections::HashMap;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Meter},
};
use shared::{Rejection, RejectionCategory};

use crate::pipeline::PipelineSnapshot;

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
    pub fn new(meter: &Meter) -> Self {
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
    pub fn emit(&mut self, snapshot: &PipelineSnapshot) {
        let stages = &snapshot.stages;
        let content = &snapshot.content;

        // Pipeline throughput — emit delta per stage
        let firehose_delta = stages.firehose.throughput.saturating_sub(self.prev_firehose);
        if firehose_delta > 0 {
            self.throughput
                .add(firehose_delta, &[KeyValue::new("stage", "firehose")]);
            self.prev_firehose = stages.firehose.throughput;
        }
        let meta_delta = stages.meta.throughput.saturating_sub(self.prev_meta);
        if meta_delta > 0 {
            self.throughput
                .add(meta_delta, &[KeyValue::new("stage", "meta")]);
            self.prev_meta = stages.meta.throughput;
        }
        let image_delta = stages.image.throughput.saturating_sub(self.prev_image);
        if image_delta > 0 {
            self.throughput
                .add(image_delta, &[KeyValue::new("stage", "image")]);
            self.prev_image = stages.image.throughput;
        }

        // Pipeline depth — emit current value
        self.depth.record(
            stages.firehose.depth as u64,
            &[KeyValue::new("stage", "firehose")],
        );
        self.depth
            .record(stages.meta.depth as u64, &[KeyValue::new("stage", "meta")]);
        self.depth
            .record(stages.image.depth as u64, &[KeyValue::new("stage", "image")]);

        // Content counters — emit delta
        let skeets_delta = content.posts.saturating_sub(self.prev_skeets);
        if skeets_delta > 0 {
            self.skeets_total.add(skeets_delta, &[]);
            self.prev_skeets = content.posts;
        }

        let images_delta = content.images.saturating_sub(self.prev_images);
        if images_delta > 0 {
            self.images_total.add(images_delta, &[]);
            self.prev_images = content.images;
        }

        let saved_delta = content.saved.saturating_sub(self.prev_saved);
        if saved_delta > 0 {
            self.saved_total.add(saved_delta, &[]);
            self.prev_saved = content.saved;
        }

        for (reason, &count) in &snapshot.rejections.by_reason {
            let prev = self.prev_rejected.get(reason).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            if delta > 0 {
                self.rejected_total
                    .add(delta, &[KeyValue::new("reason", reason.to_string())]);
                self.prev_rejected.insert(*reason, count);
            }
        }

        for (cat, &count) in &snapshot.rejections.by_category {
            let prev = self.prev_categories.get(cat).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            if delta > 0 {
                self.categories_total
                    .add(delta, &[KeyValue::new("category", cat.to_string())]);
                self.prev_categories.insert(*cat, count);
            }
        }

        for (cat, &count) in &snapshot.rejections.by_sole_category {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{ContentCounts, PipelineStages, RejectionBreakdown, StageStats};
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use test_support::{flush_and_collect, sum_counter};

    fn make_test_metrics() -> (PruneMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = PruneMetrics::new(&provider.meter("skeet_prune"));
        (metrics, provider, exporter)
    }

    fn throughput(firehose: u64, meta: u64, image: u64) -> PipelineSnapshot {
        PipelineSnapshot {
            stages: PipelineStages {
                firehose: StageStats {
                    throughput: firehose,
                    ..Default::default()
                },
                meta: StageStats {
                    throughput: meta,
                    ..Default::default()
                },
                image: StageStats {
                    throughput: image,
                    ..Default::default()
                },
            },
            ..Default::default()
        }
    }

    fn depths(firehose: usize, meta: usize, image: usize) -> PipelineSnapshot {
        PipelineSnapshot {
            stages: PipelineStages {
                firehose: StageStats {
                    depth: firehose,
                    ..Default::default()
                },
                meta: StageStats {
                    depth: meta,
                    ..Default::default()
                },
                image: StageStats {
                    depth: image,
                    ..Default::default()
                },
            },
            ..Default::default()
        }
    }

    fn content(posts: u64, images: u64, saved: u64) -> PipelineSnapshot {
        PipelineSnapshot {
            content: ContentCounts {
                posts,
                images,
                saved,
            },
            ..Default::default()
        }
    }

    fn empty_emit(metrics: &mut PruneMetrics) {
        metrics.emit(&PipelineSnapshot::default());
    }

    #[test]
    fn throughput_counter_emits_delta_only_when_increasing() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        // First emit: firehose=10 — counter should add 10.
        metrics.emit(&throughput(10, 0, 0));
        // Second emit: firehose=10 again (delta=0) — counter must NOT advance.
        metrics.emit(&throughput(10, 0, 0));
        // Third emit: firehose=15 (delta=5) — counter adds 5.
        metrics.emit(&throughput(15, 0, 0));
        let total = sum_counter(
            &provider,
            &exporter,
            "skeet_prune.pipeline.throughput",
            Some(("stage", "firehose")),
        );
        assert_eq!(total, 15, "cumulative firehose count after 10→10→15");
    }

    #[test]
    fn throughput_counter_keeps_stages_independent() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        metrics.emit(&throughput(7, 11, 13));
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(
            snap.sum_counter(
                "skeet_prune.pipeline.throughput",
                Some(("stage", "firehose"))
            ),
            7
        );
        assert_eq!(
            snap.sum_counter("skeet_prune.pipeline.throughput", Some(("stage", "meta"))),
            11
        );
        assert_eq!(
            snap.sum_counter("skeet_prune.pipeline.throughput", Some(("stage", "image"))),
            13
        );
    }

    #[test]
    fn depth_gauge_emits_current_value_each_call() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        // Gauges emit current value unconditionally — even when "delta" would be zero.
        metrics.emit(&depths(5, 7, 9));
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(
            snap.last_gauge_u64("skeet_prune.pipeline.depth", Some(("stage", "firehose"))),
            5
        );
        assert_eq!(
            snap.last_gauge_u64("skeet_prune.pipeline.depth", Some(("stage", "meta"))),
            7
        );
        assert_eq!(
            snap.last_gauge_u64("skeet_prune.pipeline.depth", Some(("stage", "image"))),
            9
        );
    }

    #[test]
    fn content_counters_emit_deltas() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        metrics.emit(&content(4, 6, 2));
        // Emit again with same totals — must not double-count.
        empty_emit(&mut metrics);
        metrics.emit(&content(4, 6, 2));
        // Then bump.
        metrics.emit(&content(5, 6, 3));
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(snap.sum_counter("skeet_prune.skeets.total", None), 5);
        assert_eq!(snap.sum_counter("skeet_prune.images.total", None), 6);
        assert_eq!(snap.sum_counter("skeet_prune.saved.total", None), 3);
    }

    #[test]
    fn rejected_counter_keyed_by_reason_emits_delta() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        let mut rejs: HashMap<Rejection, u64> = HashMap::new();
        rejs.insert(Rejection::TooMuchText, 3);
        rejs.insert(Rejection::FaceTooSmall, 1);
        let emit_reasons = |metrics: &mut PruneMetrics, rejs: &HashMap<Rejection, u64>| {
            metrics.emit(&PipelineSnapshot {
                rejections: RejectionBreakdown {
                    by_reason: rejs.clone(),
                    ..Default::default()
                },
                ..Default::default()
            });
        };
        emit_reasons(&mut metrics, &rejs);
        // Same map again — no advance.
        emit_reasons(&mut metrics, &rejs);
        // Bump TooMuchText to 7.
        rejs.insert(Rejection::TooMuchText, 7);
        emit_reasons(&mut metrics, &rejs);
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(
            snap.sum_counter(
                "skeet_prune.rejected.total",
                Some(("reason", "TooMuchText"))
            ),
            7
        );
        assert_eq!(
            snap.sum_counter(
                "skeet_prune.rejected.total",
                Some(("reason", "FaceTooSmall"))
            ),
            1
        );
    }

    #[test]
    fn category_counters_keyed_by_category_emit_delta() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        let mut cats: HashMap<RejectionCategory, u64> = HashMap::new();
        cats.insert(RejectionCategory::Face, 5);
        let mut sole: HashMap<RejectionCategory, u64> = HashMap::new();
        sole.insert(RejectionCategory::Face, 2);
        let emit_cats = |metrics: &mut PruneMetrics,
                         cats: &HashMap<RejectionCategory, u64>,
                         sole: &HashMap<RejectionCategory, u64>| {
            metrics.emit(&PipelineSnapshot {
                rejections: RejectionBreakdown {
                    by_category: cats.clone(),
                    by_sole_category: sole.clone(),
                    ..Default::default()
                },
                ..Default::default()
            });
        };
        emit_cats(&mut metrics, &cats, &sole);
        emit_cats(&mut metrics, &cats, &sole);
        cats.insert(RejectionCategory::Face, 8);
        sole.insert(RejectionCategory::Face, 3);
        emit_cats(&mut metrics, &cats, &sole);
        let snap = flush_and_collect(&provider, &exporter);
        assert_eq!(
            snap.sum_counter("skeet_prune.categories.total", Some(("category", "Face"))),
            8
        );
        assert_eq!(
            snap.sum_counter(
                "skeet_prune.categories.sole.total",
                Some(("category", "Face"))
            ),
            3
        );
    }
}
