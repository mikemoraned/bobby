use std::collections::HashMap;

use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram},
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

impl Default for LiveRefineMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveRefineMetrics {
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("skeet_live_refine");

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
