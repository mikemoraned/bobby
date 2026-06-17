use serde::{Deserialize, Serialize};
use shared::Score;
pub use shared::Threshold;

#[derive(Debug, thiserror::Error)]
#[error("{name} must be in [0.0, 1.0], got {value}")]
pub struct InvalidMetric {
    pub name: &'static str,
    pub value: f64,
}

fn validated(name: &'static str, value: f64) -> Result<f64, InvalidMetric> {
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(InvalidMetric { name, value })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Precision(f64);

impl Precision {
    /// Validating constructor for untrusted input.
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("precision", value).map(Self)
    }

    /// precision = tp / (tp + fp); `None` iff tp + fp == 0.
    /// Always in [0, 1]: tp <= tp + fp, preserved by the f64 cast.
    pub fn from_counts(true_pos: u64, false_pos: u64) -> Option<Self> {
        let denom = true_pos + false_pos;
        (denom != 0).then(|| Self(true_pos as f64 / denom as f64))
    }
}

impl From<Precision> for f64 {
    fn from(v: Precision) -> Self {
        v.0
    }
}

impl std::fmt::Display for Precision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

/// `Recall::new` rejects NaN, so `Eq` and `Ord` are sound (`f64::total_cmp` is total).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Recall(f64);

impl Recall {
    /// Validating constructor for untrusted input.
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("recall", value).map(Self)
    }

    /// recall = tp / (tp + fn); `None` iff tp + fn == 0.
    /// Always in [0, 1]: tp <= tp + fn, preserved by the f64 cast.
    pub fn from_counts(true_pos: u64, false_neg: u64) -> Option<Self> {
        let denom = true_pos + false_neg;
        (denom != 0).then(|| Self(true_pos as f64 / denom as f64))
    }
}

impl From<Recall> for f64 {
    fn from(v: Recall) -> Self {
        v.0
    }
}

impl std::fmt::Display for Recall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

impl Eq for Recall {}

impl Ord for Recall {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Recall {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct F1(f64);

impl F1 {
    /// Validating constructor for untrusted input.
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("f1", value).map(Self)
    }

    /// Harmonic mean of precision and recall (0 when both are 0).
    /// Always in [0, 1]: the harmonic mean of two [0, 1] values stays in [0, 1],
    /// guaranteed here by the `Precision`/`Recall` input types.
    pub fn harmonic(p: Precision, r: Recall) -> Self {
        let (p, r) = (f64::from(p), f64::from(r));
        let denom = p + r;
        Self(if denom == 0.0 { 0.0 } else { 2.0 * p * r / denom })
    }
}

impl From<F1> for f64 {
    fn from(v: F1) -> Self {
        v.0
    }
}

impl std::fmt::Display for F1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RocAuc(f64);

impl RocAuc {
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("roc_auc", value).map(Self)
    }
}

impl From<RocAuc> for f64 {
    fn from(v: RocAuc) -> Self {
        v.0
    }
}

impl std::fmt::Display for RocAuc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

/// The result of pinning a classifier at a target precision: the highest
/// threshold whose precision ≥ target, and the recall observed there.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PinnedPrecision {
    pub threshold: Threshold,
    pub recall: Recall,
}

/// A model score paired with the ground-truth label for that example.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelledScore {
    pub score: Score,
    pub is_positive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range() {
        assert!(Precision::new(-0.1).is_err());
        assert!(Precision::new(1.1).is_err());
    }

    #[test]
    fn accepts_endpoints() {
        assert!(Precision::new(0.0).is_ok());
        assert!(Precision::new(1.0).is_ok());
    }

    #[test]
    fn ordering_matches_underlying() {
        let lo = Recall::new(0.2).expect("valid");
        let hi = Recall::new(0.8).expect("valid");
        assert!(lo < hi);
    }

    #[test]
    fn error_carries_metric_name() {
        let err = F1::new(2.0).expect_err("out of range");
        assert_eq!(err.name, "f1");
    }
}
