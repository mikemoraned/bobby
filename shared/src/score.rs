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

    #[test]
    fn score_valid_range() {
        let s = Score::new(0.5).expect("valid");
        assert_eq!(f32::from(s), 0.5);
    }

    #[test]
    fn score_rejects_negative() {
        assert!(Score::new(-0.1).is_err());
    }

    #[test]
    fn score_rejects_over_one() {
        assert!(Score::new(1.1).is_err());
    }

    #[test]
    fn score_boundaries() {
        assert!(Score::new(0.0).is_ok());
        assert!(Score::new(1.0).is_ok());
    }

    #[test]
    fn score_roundtrips_through_string() {
        let s = Score::new(0.75).expect("valid");
        let parsed: Score = s.to_string().parse().expect("should parse");
        assert_eq!(f32::from(parsed), 0.75);
    }

    #[test]
    fn score_ordering() {
        let a = Score::new(0.3).expect("valid");
        let b = Score::new(0.9).expect("valid");
        assert!(a < b);
    }
}
