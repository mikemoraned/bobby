use std::collections::HashMap;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Meter},
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

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter, SdkMeterProvider,
        data::{AggregatedMetrics, MetricData},
    };

    fn make_test_metrics() -> (PruneMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = PruneMetrics::new(&provider.meter("skeet_prune"));
        (metrics, provider, exporter)
    }

    fn collect(
        provider: &SdkMeterProvider,
        exporter: &InMemoryMetricExporter,
    ) -> Vec<(String, Vec<(Vec<(String, String)>, u64)>)> {
        provider.force_flush().unwrap();
        let metrics = exporter.get_finished_metrics().unwrap();
        let mut out = vec![];
        for rm in &metrics {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    let points = match m.data() {
                        AggregatedMetrics::U64(MetricData::Sum(s)) => s
                            .data_points()
                            .map(|dp| {
                                let attrs = dp
                                    .attributes()
                                    .map(|kv| {
                                        (kv.key.as_str().to_string(), kv.value.as_str().to_string())
                                    })
                                    .collect();
                                (attrs, dp.value())
                            })
                            .collect(),
                        AggregatedMetrics::U64(MetricData::Gauge(g)) => g
                            .data_points()
                            .map(|dp| {
                                let attrs = dp
                                    .attributes()
                                    .map(|kv| {
                                        (kv.key.as_str().to_string(), kv.value.as_str().to_string())
                                    })
                                    .collect();
                                (attrs, dp.value())
                            })
                            .collect(),
                        _ => vec![],
                    };
                    out.push((m.name().to_string(), points));
                }
            }
        }
        exporter.reset();
        out
    }

    fn sum_for(
        snapshot: &[(String, Vec<(Vec<(String, String)>, u64)>)],
        metric: &str,
        attr: Option<(&str, &str)>,
    ) -> u64 {
        snapshot
            .iter()
            .filter(|(n, _)| n == metric)
            .flat_map(|(_, points)| points.iter())
            .filter(|(attrs, _)| {
                attr.is_none_or(|(k, v)| {
                    attrs
                        .iter()
                        .any(|(ak, av)| ak.as_str() == k && av.as_str() == v)
                })
            })
            .map(|(_, v)| v)
            .sum()
    }

    fn empty_emit(metrics: &mut PruneMetrics) {
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &HashMap::new(), &HashMap::new());
    }

    #[test]
    fn throughput_counter_emits_delta_only_when_increasing() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        // First emit: firehose=10 — counter should add 10.
        metrics.emit(10, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &HashMap::new(), &HashMap::new());
        // Second emit: firehose=10 again (delta=0) — counter must NOT advance.
        metrics.emit(10, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &HashMap::new(), &HashMap::new());
        // Third emit: firehose=15 (delta=5) — counter adds 5.
        metrics.emit(15, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &HashMap::new(), &HashMap::new());
        let snap = collect(&provider, &exporter);
        let total = sum_for(&snap, "skeet_prune.pipeline.throughput", Some(("stage", "firehose")));
        assert_eq!(total, 15, "cumulative firehose count after 10→10→15");
    }

    #[test]
    fn throughput_counter_keeps_stages_independent() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        metrics.emit(7, 11, 13, 0, 0, 0, 0, 0, 0, &HashMap::new(), &HashMap::new(), &HashMap::new());
        let snap = collect(&provider, &exporter);
        assert_eq!(sum_for(&snap, "skeet_prune.pipeline.throughput", Some(("stage", "firehose"))), 7);
        assert_eq!(sum_for(&snap, "skeet_prune.pipeline.throughput", Some(("stage", "meta"))), 11);
        assert_eq!(sum_for(&snap, "skeet_prune.pipeline.throughput", Some(("stage", "image"))), 13);
    }

    #[test]
    fn depth_gauge_emits_current_value_each_call() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        // Gauges emit current value unconditionally — even when "delta" would be zero.
        metrics.emit(0, 0, 0, 5, 7, 9, 0, 0, 0, &HashMap::new(), &HashMap::new(), &HashMap::new());
        let snap = collect(&provider, &exporter);
        assert_eq!(sum_for(&snap, "skeet_prune.pipeline.depth", Some(("stage", "firehose"))), 5);
        assert_eq!(sum_for(&snap, "skeet_prune.pipeline.depth", Some(("stage", "meta"))), 7);
        assert_eq!(sum_for(&snap, "skeet_prune.pipeline.depth", Some(("stage", "image"))), 9);
    }

    #[test]
    fn content_counters_emit_deltas() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        metrics.emit(0, 0, 0, 0, 0, 0, 4, 6, 2, &HashMap::new(), &HashMap::new(), &HashMap::new());
        // Emit again with same totals — must not double-count.
        empty_emit(&mut metrics);
        metrics.emit(0, 0, 0, 0, 0, 0, 4, 6, 2, &HashMap::new(), &HashMap::new(), &HashMap::new());
        // Then bump.
        metrics.emit(0, 0, 0, 0, 0, 0, 5, 6, 3, &HashMap::new(), &HashMap::new(), &HashMap::new());
        let snap = collect(&provider, &exporter);
        assert_eq!(sum_for(&snap, "skeet_prune.skeets.total", None), 5);
        assert_eq!(sum_for(&snap, "skeet_prune.images.total", None), 6);
        assert_eq!(sum_for(&snap, "skeet_prune.saved.total", None), 3);
    }

    #[test]
    fn rejected_counter_keyed_by_reason_emits_delta() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        let mut rejs: HashMap<Rejection, u64> = HashMap::new();
        rejs.insert(Rejection::TooMuchText, 3);
        rejs.insert(Rejection::FaceTooSmall, 1);
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &rejs, &HashMap::new(), &HashMap::new());
        // Same map again — no advance.
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &rejs, &HashMap::new(), &HashMap::new());
        // Bump TooMuchText to 7.
        rejs.insert(Rejection::TooMuchText, 7);
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &rejs, &HashMap::new(), &HashMap::new());
        let snap = collect(&provider, &exporter);
        assert_eq!(sum_for(&snap, "skeet_prune.rejected.total", Some(("reason", "TooMuchText"))), 7);
        assert_eq!(sum_for(&snap, "skeet_prune.rejected.total", Some(("reason", "FaceTooSmall"))), 1);
    }

    #[test]
    fn category_counters_keyed_by_category_emit_delta() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        let mut cats: HashMap<RejectionCategory, u64> = HashMap::new();
        cats.insert(RejectionCategory::Face, 5);
        let mut sole: HashMap<RejectionCategory, u64> = HashMap::new();
        sole.insert(RejectionCategory::Face, 2);
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &cats, &sole);
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &cats, &sole);
        cats.insert(RejectionCategory::Face, 8);
        sole.insert(RejectionCategory::Face, 3);
        metrics.emit(0, 0, 0, 0, 0, 0, 0, 0, 0, &HashMap::new(), &cats, &sole);
        let snap = collect(&provider, &exporter);
        assert_eq!(
            sum_for(&snap, "skeet_prune.categories.total", Some(("category", "Face"))),
            8
        );
        assert_eq!(
            sum_for(&snap, "skeet_prune.categories.sole.total", Some(("category", "Face"))),
            3
        );
    }
}
