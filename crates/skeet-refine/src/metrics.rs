use std::collections::HashMap;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter},
};

/// OTel metrics emitted by skeet-live-refine at the end of each poll tick.
pub struct LiveRefineMetrics {
    images_unscored: Counter<u64>,
    images_scored: Counter<u64>,
    images_errors: Counter<u64>,
    scores_hist: Histogram<f64>,

    // Tracks the cumulative totals emitted so far; deltas are added each tick.
    prev_unscored: u64,
    prev_scored: u64,
    prev_errors: HashMap<String, u64>,
}

impl LiveRefineMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            images_unscored: meter
                .u64_counter("skeet_live_refine.images.unscored")
                .with_description("Cumulative images found unscored at the start of each tick")
                .with_unit("images")
                .build(),
            images_scored: meter
                .u64_counter("skeet_live_refine.images.scored")
                .with_description("Cumulative images successfully scored")
                .with_unit("images")
                .build(),
            images_errors: meter
                .u64_counter("skeet_live_refine.images.errors")
                .with_description("Cumulative scoring errors by reason")
                .with_unit("images")
                .build(),
            scores_hist: meter
                .f64_histogram("skeet_live_refine.scores")
                .with_description("Score distribution of refined images")
                .with_boundaries(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9])
                .build(),
            prev_unscored: 0,
            prev_scored: 0,
            prev_errors: HashMap::new(),
        }
    }

    /// Emit metrics for one tick. Counters receive the delta since the last call;
    /// the histogram receives direct observations from this tick.
    pub fn emit(
        &mut self,
        unscored_total: u64,
        scored_total: u64,
        error_totals: &HashMap<String, u64>,
        tick_scores: &[f64],
    ) {
        let unscored_delta = unscored_total.saturating_sub(self.prev_unscored);
        if unscored_delta > 0 {
            self.images_unscored.add(unscored_delta, &[]);
            self.prev_unscored = unscored_total;
        }

        let scored_delta = scored_total.saturating_sub(self.prev_scored);
        if scored_delta > 0 {
            self.images_scored.add(scored_delta, &[]);
            self.prev_scored = scored_total;
        }

        for (reason, &count) in error_totals {
            let prev = self.prev_errors.get(reason).copied().unwrap_or(0);
            let delta = count.saturating_sub(prev);
            if delta > 0 {
                self.images_errors
                    .add(delta, &[KeyValue::new("reason", reason.clone())]);
                self.prev_errors.insert(reason.clone(), count);
            }
        }

        for &score in tick_scores {
            self.scores_hist.record(score, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};
    use test_support::{histogram_observation_count, sum_counter};

    fn make_test_metrics() -> (LiveRefineMetrics, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = LiveRefineMetrics::new(&provider.meter("skeet_live_refine"));
        (metrics, provider, exporter)
    }

    #[test]
    fn unscored_and_scored_counters_advance_only_on_increase() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        metrics.emit(5, 3, &HashMap::new(), &[]);
        // Same totals → no advance.
        metrics.emit(5, 3, &HashMap::new(), &[]);
        // Bump.
        metrics.emit(8, 5, &HashMap::new(), &[]);
        assert_eq!(
            sum_counter(
                &provider,
                &exporter,
                "skeet_live_refine.images.unscored",
                None
            ),
            8
        );
        // Need a fresh provider read after reset for second metric.
        let (mut metrics2, provider2, exporter2) = make_test_metrics();
        metrics2.emit(5, 3, &HashMap::new(), &[]);
        metrics2.emit(5, 3, &HashMap::new(), &[]);
        metrics2.emit(8, 5, &HashMap::new(), &[]);
        assert_eq!(
            sum_counter(
                &provider2,
                &exporter2,
                "skeet_live_refine.images.scored",
                None
            ),
            5
        );
    }

    #[test]
    fn error_counter_emits_per_reason_delta() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        let mut errs: HashMap<String, u64> = HashMap::new();
        errs.insert("Completion".to_string(), 2);
        errs.insert("ParseScore".to_string(), 1);
        metrics.emit(0, 0, &errs, &[]);
        metrics.emit(0, 0, &errs, &[]); // no advance
        errs.insert("Completion".to_string(), 5);
        metrics.emit(0, 0, &errs, &[]);
        assert_eq!(
            sum_counter(
                &provider,
                &exporter,
                "skeet_live_refine.images.errors",
                Some(("reason", "Completion"))
            ),
            5
        );
        let (mut m2, p2, e2) = make_test_metrics();
        let mut errs2: HashMap<String, u64> = HashMap::new();
        errs2.insert("ParseScore".to_string(), 1);
        m2.emit(0, 0, &errs2, &[]);
        assert_eq!(
            sum_counter(
                &p2,
                &e2,
                "skeet_live_refine.images.errors",
                Some(("reason", "ParseScore"))
            ),
            1
        );
    }

    #[test]
    fn histogram_records_one_observation_per_tick_score() {
        let (mut metrics, provider, exporter) = make_test_metrics();
        metrics.emit(0, 0, &HashMap::new(), &[0.1, 0.5, 0.9]);
        assert_eq!(
            histogram_observation_count(&provider, &exporter, "skeet_live_refine.scores", None),
            3
        );
    }
}
