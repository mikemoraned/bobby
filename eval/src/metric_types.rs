use serde::{Deserialize, Serialize};
use shared::Score;

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
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("precision", value).map(Self)
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
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("recall", value).map(Self)
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
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("f1", value).map(Self)
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

/// `Threshold::new` rejects NaN, so `Eq` and `Ord` are sound (`f64::total_cmp` is total).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Threshold(f64);

impl Threshold {
    pub fn new(value: f64) -> Result<Self, InvalidMetric> {
        validated("threshold", value).map(Self)
    }
}

impl From<Threshold> for f64 {
    fn from(v: Threshold) -> Self {
        v.0
    }
}

impl std::fmt::Display for Threshold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.3}", self.0)
    }
}

impl From<Score> for Threshold {
    fn from(s: Score) -> Self {
        // Score is validated to [0.0, 1.0], so Threshold::new is infallible here.
        Self::new(f64::from(s)).expect("Score is in [0.0, 1.0]")
    }
}

impl Eq for Threshold {}

impl Ord for Threshold {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Threshold {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
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
