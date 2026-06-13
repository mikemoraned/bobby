use eval::{PinnedPrecision, Recall, Threshold};
use shared::refine_model::{ModelName, ModelProvider, RefineModel, RefinePrompt};

/// Threshold used during the training loop to score in-loop F1 — the prompt
/// with the best F1 at this threshold becomes the candidate. The deployed
/// threshold is decided separately after the loop by pinning at the baseline's
/// precision floor.
const TRAINING_LOOP_THRESHOLD_F64: f64 = 0.5;

/// Maximum tolerated drop in recall (absolute) at the baseline's precision
/// floor before the candidate is rejected.
pub const GATE_RECALL_TOLERANCE: f64 = 0.01;

#[allow(clippy::expect_used)] // the constant is a compile-time literal in [0, 1]
pub fn training_loop_threshold() -> Threshold {
    Threshold::new(TRAINING_LOOP_THRESHOLD_F64).expect("0.5 is in [0, 1]")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    Accepted,
    Rejected,
}

/// Decide whether the candidate clears the baseline. At the baseline's
/// precision floor, the candidate's recall must be within
/// `GATE_RECALL_TOLERANCE` absolute of the baseline's recall.
pub fn evaluate_gate(pinned: Option<PinnedPrecision>, baseline_recall: Recall) -> GateOutcome {
    match pinned {
        Some(p) if f64::from(p.recall) >= f64::from(baseline_recall) - GATE_RECALL_TOLERANCE => {
            GateOutcome::Accepted
        }
        _ => GateOutcome::Rejected,
    }
}

/// Build the `RefineModel` that, on Accept, will be appended to the registry
/// and labelled `production`.
pub fn build_candidate_model(model_name: &str, prompt: &str, threshold: Threshold) -> RefineModel {
    RefineModel {
        model_provider: ModelProvider::openai(),
        model_name: ModelName::new(model_name),
        prompt: RefinePrompt::new(prompt),
        decision_threshold: threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::refine_model::{Label, RefineModels};

    #[test]
    fn gate_accepts_when_recall_within_tolerance() {
        let baseline_recall = Recall::new(0.70).expect("valid");
        let pinned = PinnedPrecision {
            threshold: Threshold::new(0.5).expect("valid"),
            recall: Recall::new(f64::from(baseline_recall) - GATE_RECALL_TOLERANCE / 2.0)
                .expect("valid"),
        };
        assert_eq!(
            evaluate_gate(Some(pinned), baseline_recall),
            GateOutcome::Accepted
        );
    }

    #[test]
    fn gate_rejects_when_recall_drops_below_tolerance() {
        let baseline_recall = Recall::new(0.70).expect("valid");
        let pinned = PinnedPrecision {
            threshold: Threshold::new(0.5).expect("valid"),
            recall: Recall::new(f64::from(baseline_recall) - GATE_RECALL_TOLERANCE * 1.5)
                .expect("valid"),
        };
        assert_eq!(
            evaluate_gate(Some(pinned), baseline_recall),
            GateOutcome::Rejected
        );
    }

    #[test]
    fn gate_rejects_when_no_qualifying_threshold() {
        assert_eq!(
            evaluate_gate(None, Recall::new(0.5).expect("valid")),
            GateOutcome::Rejected
        );
    }

    #[test]
    fn inserting_candidate_makes_it_resolvable_by_production_label() {
        let candidate = build_candidate_model(
            "gpt-4o",
            "any prompt",
            Threshold::new(0.5).expect("valid"),
        );
        let mut models = RefineModels::new();
        models.insert(candidate.clone(), &[Label::production()]);

        let resolved = models
            .by_label(&Label::production())
            .expect("production label resolves");
        assert_eq!(resolved, &candidate);
    }
}
