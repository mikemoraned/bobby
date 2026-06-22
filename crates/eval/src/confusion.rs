use serde::{Deserialize, Serialize};

use crate::metric_types::{F1, Precision, Recall};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfusionMatrix {
    pub true_pos: u64,
    pub false_pos: u64,
    pub true_neg: u64,
    pub false_neg: u64,
}

impl ConfusionMatrix {
    pub const fn record(&mut self, should_be_positive: bool, predicted_positive: bool) {
        match (should_be_positive, predicted_positive) {
            (true, true) => self.true_pos += 1,
            (false, true) => self.false_pos += 1,
            (false, false) => self.true_neg += 1,
            (true, false) => self.false_neg += 1,
        }
    }

    /// `None` when no positive predictions were made (precision is undefined).
    pub fn precision(&self) -> Option<Precision> {
        Precision::from_counts(self.true_pos, self.false_pos)
    }

    /// `None` when no actual positives exist (recall is undefined).
    pub fn recall(&self) -> Option<Recall> {
        Recall::from_counts(self.true_pos, self.false_neg)
    }

    /// `None` when either precision or recall is undefined.
    pub fn f1(&self) -> Option<F1> {
        Some(F1::harmonic(self.precision()?, self.recall()?))
    }

    pub const fn total(&self) -> u64 {
        self.true_pos + self.false_pos + self.true_neg + self.false_neg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_classifier() {
        let mut m = ConfusionMatrix::default();
        m.record(true, true);
        m.record(false, false);
        assert_eq!(f64::from(m.precision().expect("defined")), 1.0);
        assert_eq!(f64::from(m.recall().expect("defined")), 1.0);
        assert_eq!(f64::from(m.f1().expect("defined")), 1.0);
    }

    #[test]
    fn only_false_positives_makes_recall_undefined() {
        let mut m = ConfusionMatrix::default();
        m.record(false, true);
        assert_eq!(f64::from(m.precision().expect("defined")), 0.0);
        assert_eq!(m.recall(), None);
        assert_eq!(m.f1(), None);
    }

    #[test]
    fn empty_matrix_makes_all_metrics_undefined() {
        let m = ConfusionMatrix::default();
        assert_eq!(m.precision(), None);
        assert_eq!(m.recall(), None);
        assert_eq!(m.f1(), None);
    }
}
