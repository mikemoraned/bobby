//! Per-tick state for the live-refine loop.
//!
//! Each tick fetches a batch, scores it, and records outcomes into a
//! [`TickAccumulator`]. At the end of the tick those outcomes are folded into
//! the cross-tick [`RunningTotals`] that drive cumulative OTel counters.

use std::collections::HashMap;

use shared::{ImageId, ModelVersion, Score};
use tracing::{error, info};

use crate::batch::ScoreOutcomes;

/// Failure modes visible to the live-refine dispatcher after retries.
///
/// Transient `Completion` errors are already absorbed by the resilient
/// wrapper upstream — anything that surfaces here means the wrapper has
/// given up.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum ScoringFailure {
    #[error("scoring fell back after exhausted retries; image left unscored for next tick")]
    FallbackAfterRetries,
}

impl ScoringFailure {
    pub const fn as_label(&self) -> &'static str {
        match self {
            Self::FallbackAfterRetries => "FallbackAfterRetries",
        }
    }
}

/// Running totals accumulated across all ticks, used to drive OTel counters.
pub struct RunningTotals {
    pub unscored: u64,
    pub scored: u64,
    pub errors: HashMap<String, u64>,
}

impl RunningTotals {
    pub fn new() -> Self {
        Self {
            unscored: 0,
            scored: 0,
            errors: HashMap::new(),
        }
    }

    pub fn absorb_tick(&mut self, unscored_count: u64, acc: &TickAccumulator) {
        self.unscored += unscored_count;
        self.scored += acc.pending_scores.len() as u64;
        acc.merge_errors_into(&mut self.errors);
    }
}

impl Default for RunningTotals {
    fn default() -> Self {
        Self::new()
    }
}

/// Mutable state accumulated within a single tick.
pub struct TickAccumulator {
    pub pending_scores: Vec<(ImageId, Score, ModelVersion)>,
    pub errors: HashMap<String, u64>,
}

impl TickAccumulator {
    pub fn new() -> Self {
        Self {
            pending_scores: Vec::new(),
            errors: HashMap::new(),
        }
    }

    /// Record per-image outcomes from a scored batch: log each, push successes
    /// onto `pending_scores`, and bump per-reason failure counts.
    ///
    /// Failures here represent the wrapper-level outcome (`FallbackAfterRetries`),
    /// not lower-level [`crate::refining::RefineError`] variants — those have
    /// already been absorbed by retries upstream.
    pub fn record_outcomes(
        &mut self,
        outcomes: ScoreOutcomes<ScoringFailure>,
        model_version: &ModelVersion,
    ) {
        for (id, score) in outcomes.successes {
            info!(image_id = %id, %score, "refined");
            self.pending_scores
                .push((id, score, model_version.clone()));
        }
        for (id, e) in outcomes.failures {
            error!(image_id = %id, error = %e, "scoring did not produce a saveable score");
            *self.errors.entry(e.as_label().to_string()).or_default() += 1;
        }
    }

    pub fn merge_errors_into(&self, totals: &mut HashMap<String, u64>) {
        for (reason, count) in &self.errors {
            *totals.entry(reason.clone()).or_default() += count;
        }
    }

    /// Extract scores as `f64` observations for the histogram.
    pub fn scores(&self) -> Vec<f64> {
        self.pending_scores
            .iter()
            .map(|(_, s, _)| f64::from(*s))
            .collect()
    }

    /// Number of unscored images that did *not* receive a successful score
    /// this tick (i.e. failed or were dropped). Used for the "remaining" log.
    pub const fn remaining(&self, unscored_count: u64) -> usize {
        unscored_count as usize - self.pending_scores.len()
    }
}

impl Default for TickAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(s: &str) -> ModelVersion {
        ModelVersion::from(s)
    }

    fn score(v: f32) -> Score {
        Score::new(v).expect("score in 0..1")
    }

    fn id(seed: u8) -> ImageId {
        use image::{ImageBuffer, Rgb};
        let img = image::DynamicImage::ImageRgb8(ImageBuffer::from_pixel(
            u32::from(seed) + 1,
            1,
            Rgb([0u8, 0, 0]),
        ));
        ImageId::from_image(&img)
    }

    #[test]
    fn record_outcomes_pushes_successes_with_supplied_model_version() {
        let mut acc = TickAccumulator::new();
        let v = version("model@v1");
        let outcomes = ScoreOutcomes::<ScoringFailure> {
            successes: vec![(id(1), score(0.4)), (id(2), score(0.7))],
            failures: vec![],
        };
        acc.record_outcomes(outcomes, &v);
        assert_eq!(acc.pending_scores.len(), 2);
        for (_, _, mv) in &acc.pending_scores {
            assert_eq!(*mv, v);
        }
        assert!(acc.errors.is_empty());
    }

    #[test]
    fn record_outcomes_bumps_fallback_count_for_each_failure() {
        let mut acc = TickAccumulator::new();
        let v = version("model@v1");
        let outcomes = ScoreOutcomes::<ScoringFailure> {
            successes: vec![],
            failures: vec![
                (id(1), ScoringFailure::FallbackAfterRetries),
                (id(2), ScoringFailure::FallbackAfterRetries),
                (id(3), ScoringFailure::FallbackAfterRetries),
            ],
        };
        acc.record_outcomes(outcomes, &v);
        assert_eq!(acc.errors.get("FallbackAfterRetries"), Some(&3));
        assert!(acc.pending_scores.is_empty());
    }

    #[test]
    fn record_outcomes_increments_existing_error_counts_rather_than_replacing() {
        let mut acc = TickAccumulator::new();
        acc.errors.insert("FallbackAfterRetries".to_string(), 3);
        let v = version("model@v1");
        let outcomes = ScoreOutcomes::<ScoringFailure> {
            successes: vec![],
            failures: vec![(id(1), ScoringFailure::FallbackAfterRetries)],
        };
        acc.record_outcomes(outcomes, &v);
        // 3 + 1 = 4 — kills `+= -=` and `+= *=` mutants on the error increment.
        assert_eq!(acc.errors.get("FallbackAfterRetries"), Some(&4));
    }

    #[test]
    fn merge_errors_into_adds_per_reason_counts_to_existing_totals() {
        let mut acc = TickAccumulator::new();
        acc.errors.insert("Completion".to_string(), 2);
        acc.errors.insert("ParseScore".to_string(), 1);
        let mut totals: HashMap<String, u64> = HashMap::new();
        totals.insert("Completion".to_string(), 5);
        acc.merge_errors_into(&mut totals);
        // 5 + 2 = 7 — kills `+= -=` and `+= *=` mutants.
        assert_eq!(totals.get("Completion"), Some(&7));
        // Fresh reason carried over verbatim.
        assert_eq!(totals.get("ParseScore"), Some(&1));
    }

    #[test]
    fn scores_returns_one_f64_per_pending_score() {
        let mut acc = TickAccumulator::new();
        let v = version("model@v1");
        acc.pending_scores
            .push((id(1), score(0.25), v.clone()));
        acc.pending_scores
            .push((id(2), score(0.75), v));
        let scores = acc.scores();
        // Kills `vec![]`, `vec![0.0]`, `vec![1.0]`, `vec![-1.0]` mutants.
        assert_eq!(scores.len(), 2);
        assert!((scores[0] - 0.25).abs() < 1e-6);
        assert!((scores[1] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn remaining_subtracts_successful_scores_from_unscored_count() {
        let mut acc = TickAccumulator::new();
        let v = version("model@v1");
        for i in 0..3u8 {
            acc.pending_scores.push((id(i), score(0.5), v.clone()));
        }
        // 10 - 3 = 7 — kills `- +` and `- /` mutants.
        assert_eq!(acc.remaining(10), 7);
    }

    #[test]
    fn absorb_tick_adds_unscored_count_to_running_total() {
        let mut totals = RunningTotals::new();
        totals.unscored = 100;
        let acc = TickAccumulator::new();
        totals.absorb_tick(7, &acc);
        // 100 + 7 = 107 — kills `+= -=` and `+= *=` on unscored.
        assert_eq!(totals.unscored, 107);
    }

    #[test]
    fn absorb_tick_adds_pending_scores_count_to_running_total() {
        let mut totals = RunningTotals::new();
        totals.scored = 50;
        let mut acc = TickAccumulator::new();
        let v = version("model@v1");
        for i in 0..4u8 {
            acc.pending_scores.push((id(i), score(0.5), v.clone()));
        }
        totals.absorb_tick(0, &acc);
        // 50 + 4 = 54 — kills `+= -=` and `+= *=` on scored.
        assert_eq!(totals.scored, 54);
    }

    #[test]
    fn absorb_tick_merges_per_reason_errors_into_running_total() {
        let mut totals = RunningTotals::new();
        totals.errors.insert("Completion".to_string(), 1);
        let mut acc = TickAccumulator::new();
        acc.errors.insert("Completion".to_string(), 2);
        acc.errors.insert("ParseScore".to_string(), 5);
        totals.absorb_tick(0, &acc);
        assert_eq!(totals.errors.get("Completion"), Some(&3));
        assert_eq!(totals.errors.get("ParseScore"), Some(&5));
    }
}
