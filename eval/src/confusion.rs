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
        let denom = self.true_pos + self.false_pos;
        if denom == 0 {
            return None;
        }
        let value = self.true_pos as f64 / denom as f64;
        Some(Precision::new(value).expect("precision in [0, 1] by construction"))
    }

    /// `None` when no actual positives exist (recall is undefined).
    pub fn recall(&self) -> Option<Recall> {
        let denom = self.true_pos + self.false_neg;
        if denom == 0 {
            return None;
        }
        let value = self.true_pos as f64 / denom as f64;
        Some(Recall::new(value).expect("recall in [0, 1] by construction"))
    }

    /// `None` when either precision or recall is undefined.
    pub fn f1(&self) -> Option<F1> {
        let p: f64 = self.precision()?.into();
        let r: f64 = self.recall()?.into();
        let denom = p + r;
        let value = if denom == 0.0 { 0.0 } else { 2.0 * p * r / denom };
        Some(F1::new(value).expect("f1 in [0, 1] by construction"))
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
