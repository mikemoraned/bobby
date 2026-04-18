/// A score in the range 0.0–1.0, where 1.0 is the best match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score(f32);

#[derive(Debug, Clone, thiserror::Error)]
#[error("score must be between 0.0 and 1.0, got {0}")]
pub struct InvalidScore(f32);

impl Score {
    pub fn new(value: f32) -> Result<Self, InvalidScore> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidScore(value))
        }
    }
}

impl From<Score> for f32 {
    fn from(score: Score) -> Self {
        score.0
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

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
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
    }
}
