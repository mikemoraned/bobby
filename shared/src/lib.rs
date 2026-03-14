#![warn(clippy::all, clippy::nursery)]

use serde::Deserialize;

/// A percentage value in the range 0.0–100.0.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
pub struct Percentage(f32);

impl Percentage {
    pub fn new(value: f32) -> Self {
        assert!(
            (0.0..=100.0).contains(&value),
            "percentage must be between 0.0 and 100.0, got {value}"
        );
        Self(value)
    }

    pub const fn value(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for Percentage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1}%", self.0)
    }
}

impl PartialEq for Percentage {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl PartialOrd for Percentage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rejection {
    FaceTooSmall,
    FaceTooLarge,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FaceTooSmall => write!(f, "FaceTooSmall"),
            Self::FaceTooLarge => write!(f, "FaceTooLarge"),
        }
    }
}

impl std::str::FromStr for Rejection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "FaceTooSmall" => Ok(Self::FaceTooSmall),
            "FaceTooLarge" => Ok(Self::FaceTooLarge),
            other => Err(format!("unknown rejection: {other}")),
        }
    }
}

/// Configuration for archetype classification thresholds.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ArchetypeConfig {
    pub min_face_area_pct: Percentage,
    pub max_face_area_pct: Percentage,
}

/// Result of classifying an image: either an archetype (quadrant) or rejection reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    Accepted(Quadrant),
    Rejected(Vec<Rejection>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quadrant {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl std::fmt::Display for Quadrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopLeft => write!(f, "TOP_LEFT"),
            Self::TopRight => write!(f, "TOP_RIGHT"),
            Self::BottomLeft => write!(f, "BOTTOM_LEFT"),
            Self::BottomRight => write!(f, "BOTTOM_RIGHT"),
        }
    }
}

impl std::str::FromStr for Quadrant {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "TOP_LEFT" => Ok(Self::TopLeft),
            "TOP_RIGHT" => Ok(Self::TopRight),
            "BOTTOM_LEFT" => Ok(Self::BottomLeft),
            "BOTTOM_RIGHT" => Ok(Self::BottomRight),
            other => Err(format!("unknown quadrant: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentage_valid_range() {
        let p = Percentage::new(50.0);
        assert_eq!(p.value(), 50.0);
    }

    #[test]
    #[should_panic(expected = "percentage must be between")]
    fn percentage_rejects_negative() {
        Percentage::new(-1.0);
    }

    #[test]
    #[should_panic(expected = "percentage must be between")]
    fn percentage_rejects_over_100() {
        Percentage::new(100.1);
    }

    #[test]
    fn percentage_ordering() {
        let a = Percentage::new(10.0);
        let b = Percentage::new(60.0);
        assert!(a < b);
    }

    #[test]
    fn rejection_roundtrips_through_string() {
        for r in [Rejection::FaceTooSmall, Rejection::FaceTooLarge] {
            let s = r.to_string();
            let parsed: Rejection = s.parse().expect("should parse");
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn quadrant_roundtrips_through_string() {
        for q in [
            Quadrant::TopLeft,
            Quadrant::TopRight,
            Quadrant::BottomLeft,
            Quadrant::BottomRight,
        ] {
            let s = q.to_string();
            let parsed: Quadrant = s.parse().expect("should parse");
            assert_eq!(parsed, q);
        }
    }
}
