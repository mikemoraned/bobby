use smartcore::metrics::roc_auc_score as smartcore_roc_auc_score;

use crate::confusion::ConfusionMatrix;
use crate::metric_types::{LabelledScore, PinnedPrecision, Precision, RocAuc, Threshold};

/// Computes the ROC-AUC score via `smartcore::metrics::roc_auc_score`.
///
/// Returns `None` if either class is empty (AUC is undefined).
#[allow(clippy::expect_used)] // smartcore returns AUC in [0.0, 1.0]
pub fn roc_auc_score(labelled: &[LabelledScore]) -> Option<RocAuc> {
    let n_pos = labelled.iter().filter(|l| l.is_positive).count();
    let n_neg = labelled.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return None;
    }

    let y_true: Vec<f64> = labelled
        .iter()
        .map(|l| if l.is_positive { 1.0 } else { 0.0 })
        .collect();
    let y_pred: Vec<f64> = labelled.iter().map(|l| f64::from(l.score)).collect();

    let raw = smartcore_roc_auc_score(&y_true, &y_pred);
    Some(RocAuc::new(raw).expect("smartcore returns AUC in [0.0, 1.0]"))
}

/// Find a classifier threshold whose precision meets `target_precision`,
/// returning that threshold and the recall observed there. `None` if no threshold qualifies.
///
/// Candidates are the **distinct operating points** for `labelled` — i.e. the unique scores
/// treated as thresholds. Between two adjacent observed scores the confusion matrix is
/// identical, so this gives complete, non-redundant coverage of every operating point the
/// classifier can reach. When multiple thresholds qualify, the one with the **highest
/// recall** is returned — the most recall-maximising operating point that still meets the
/// precision floor.
///
/// Used to compare classifiers fairly: pinning at a baseline's precision lets us read off
/// recall at the same quality bar regardless of how the candidate's scores are calibrated.
pub fn pin_at_precision(
    labelled: &[LabelledScore],
    target_precision: Precision,
) -> Option<PinnedPrecision> {
    let mut distinct_operating_points: Vec<Threshold> =
        labelled.iter().map(|l| l.score.into()).collect();
    distinct_operating_points.sort_by(|a, b| b.cmp(a));
    distinct_operating_points.dedup();

    distinct_operating_points
        .into_iter()
        .filter_map(|operating_point| {
            let matrix = confusion_at(labelled, operating_point);
            match matrix.precision() {
                None => None,
                Some(p) if p < target_precision => None,
                Some(_) => matrix.recall().map(|recall| PinnedPrecision {
                    threshold: operating_point,
                    recall,
                }),
            }
        })
        .max_by_key(|p| p.recall)
}

pub fn confusion_at(labelled: &[LabelledScore], threshold: Threshold) -> ConfusionMatrix {
    let mut matrix = ConfusionMatrix::default();
    for &l in labelled {
        matrix.record(l.is_positive, Threshold::from(l.score) >= threshold);
    }
    matrix
}

#[cfg(test)]
mod tests {
    use shared::Score;

    use super::*;

    fn ls(score: f32, is_positive: bool) -> LabelledScore {
        LabelledScore {
            score: Score::new(score).expect("test value in [0, 1]"),
            is_positive,
        }
    }

    fn data(items: &[(f32, bool)]) -> Vec<LabelledScore> {
        items.iter().map(|(v, l)| ls(*v, *l)).collect()
    }

    fn auc_value(d: &[LabelledScore]) -> f64 {
        roc_auc_score(d).expect("both classes present").into()
    }

    #[test]
    fn roc_auc_perfect_classifier() {
        let d = data(&[(0.9, true), (0.8, true), (0.2, false), (0.1, false)]);
        assert!((auc_value(&d) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn roc_auc_random_classifier() {
        let d = data(&[(0.9, false), (0.8, true), (0.6, true), (0.2, false)]);
        assert!((auc_value(&d) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn roc_auc_inverted_classifier() {
        let d = data(&[(0.9, false), (0.8, false), (0.2, true), (0.1, true)]);
        assert!((auc_value(&d) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn roc_auc_all_same_class_returns_none() {
        let d = data(&[(0.9, true), (0.8, true)]);
        assert_eq!(roc_auc_score(&d), None);
    }

    #[test]
    fn pin_picks_qualifying_operating_point_with_max_recall() {
        // Two qualifying thresholds at target precision = 1.0:
        //   T = 0.9: TP=1, FP=0  → precision=1.0, recall=1/3
        //   T = 0.8: TP=2, FP=0  → precision=1.0, recall=2/3
        // T = 0.7 brings in a false positive, dropping precision below the floor.
        // We want the qualifying point with HIGHER recall (= 0.8), not the one with
        // higher threshold (= 0.9).
        let d = data(&[
            (0.9, true),
            (0.8, true),
            (0.7, false),
            (0.3, false),
            (0.2, true),
            (0.1, false),
        ]);
        let pinned =
            pin_at_precision(&d, Precision::new(1.0).expect("valid")).expect("threshold exists");
        let threshold: f64 = pinned.threshold.into();
        let recall: f64 = pinned.recall.into();
        assert!((threshold - 0.8).abs() < 1e-6, "expected 0.8, got {threshold}");
        assert!((recall - 2.0 / 3.0).abs() < 1e-6, "expected 2/3, got {recall}");
    }

    #[test]
    fn pin_precision_no_qualifying_threshold_returns_none() {
        let d = data(&[(0.9, false), (0.1, true)]);
        assert_eq!(
            pin_at_precision(&d, Precision::new(0.99).expect("valid")),
            None
        );
    }
}
