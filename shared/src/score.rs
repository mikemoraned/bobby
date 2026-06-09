use serde::{Deserialize, Serialize};

/// A score in the range 0.0–1.0, where 1.0 is the best match.
///
/// `Score::new` rejects NaN (NaN is not in any range), so the type guarantees
/// non-NaN — which is why `Eq` and `Ord` can be implemented soundly even though
/// `f32` itself does not provide them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score(f32);

#[derive(Debug, Clone, thiserror::Error)]
#[error("score must be between 0.0 and 1.0, got {0}")]
pub struct InvalidScore(f32);

impl Score {
    /// Validating constructor for untrusted input.
    pub fn new(value: f32) -> Result<Self, InvalidScore> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidScore(value))
        }
    }

    /// The lowest score, 0.0 — infallible since the constant is in range.
    pub const fn zero() -> Self {
        Self(0.0)
    }
}

impl From<Score> for f32 {
    fn from(score: Score) -> Self {
        score.0
    }
}

impl From<Score> for f64 {
    fn from(score: Score) -> Self {
        Self::from(score.0)
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

impl std::str::FromStr for Score {
    type Err = InvalidScore;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value: f32 = s.parse().map_err(|_| InvalidScore(f32::NAN))?;
        Self::new(value)
    }
}

impl Eq for Score {}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A score rescaled so a model's decision threshold maps to 0.5, making scores
/// produced by models with different thresholds comparable.
///
/// Always in `[0.0, 1.0]` and non-NaN (the constructor guards both), so `Eq` and
/// `Ord` are sound via `f32::total_cmp` even though `f32` is not. Construction is
/// `From<NormalizedScore> for f32` to read it back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedScore(f32);

#[derive(Debug, Clone, thiserror::Error)]
#[error("normalized score must be between 0.0 and 1.0, got {0}")]
pub struct InvalidNormalizedScore(f32);

impl NormalizedScore {
    /// Validating constructor: rejects NaN and any value outside `[0.0, 1.0]`
    /// rather than clamping, so an out-of-range input surfaces as an error.
    pub fn new(value: f32) -> Result<Self, InvalidNormalizedScore> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidNormalizedScore(value))
        }
    }
}

impl From<NormalizedScore> for f32 {
    fn from(score: NormalizedScore) -> Self {
        score.0
    }
}

impl Eq for NormalizedScore {}

impl Ord for NormalizedScore {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl PartialOrd for NormalizedScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A threshold in the range 0.0–1.0.
///
/// `Threshold::new` rejects NaN and out-of-range values, so `Eq` and `Ord`
/// are implemented soundly via `f64::total_cmp`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Threshold(f64);

#[derive(Debug, Clone, thiserror::Error)]
#[error("threshold must be in [0.0, 1.0], got {0}")]
pub struct InvalidThreshold(f64);

impl Threshold {
    pub fn new(value: f64) -> Result<Self, InvalidThreshold> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidThreshold(value))
        }
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
        // Score is validated to [0.0, 1.0], the same range Threshold accepts, so this
        // direct construction upholds Threshold's invariant without a fallible parse.
        Self(f64::from(s))
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// `Score::new(x).is_ok()` iff `0.0 ≤ x ≤ 1.0` (NaN and out-of-range both rejected).
        #[test]
        fn score_validity(x in proptest::num::f32::ANY) {
            let result = Score::new(x);
            let expected_valid = (0.0..=1.0).contains(&x);
            prop_assert_eq!(result.is_ok(), expected_valid);
        }

        /// Display uses `{:.2}` (2 decimal places). Generate scores in hundredths so the
        /// roundtrip is exact: `Score(i/100) → "{:.2}" → parse → same value`.
        #[test]
        fn score_roundtrip(i in 0u32..=100u32) {
            let score = Score::new(i as f32 / 100.0).expect("hundredths are always in [0, 1]");
            let parsed: Score = score.to_string().parse().expect("display output is valid");
            prop_assert_eq!(score.to_string(), parsed.to_string());
        }

        #[test]
        fn score_ordering_matches_f32(a in 0.0f32..=1.0f32, b in 0.0f32..=1.0f32) {
            let sa = Score::new(a).expect("valid");
            let sb = Score::new(b).expect("valid");
            prop_assert_eq!(sa.partial_cmp(&sb), a.partial_cmp(&b));
        }

        /// `NormalizedScore::new(x).is_ok()` iff `0.0 ≤ x ≤ 1.0` (NaN and
        /// out-of-range both rejected, never clamped).
        #[test]
        fn normalized_validity(x in proptest::num::f32::ANY) {
            let expected_valid = (0.0..=1.0).contains(&x);
            prop_assert_eq!(NormalizedScore::new(x).is_ok(), expected_valid);
        }

        #[test]
        fn normalized_ordering_matches_f32(a in 0.0f32..=1.0f32, b in 0.0f32..=1.0f32) {
            let na = NormalizedScore::new(a).expect("valid");
            let nb = NormalizedScore::new(b).expect("valid");
            prop_assert_eq!(na.cmp(&nb), a.total_cmp(&b));
        }
    }
}
