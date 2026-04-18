//! Quality bands for appraising skeets and images.

use crate::Score;

/// A quality band that a skeet or image falls into.
///
/// Bands are ordered worst to best so that `Ord` yields the natural ordering:
/// `Low < MediumLow < MediumHigh < HighQuality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    Low,
    MediumLow,
    MediumHigh,
    HighQuality,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown band: {0}")]
pub struct ParseBandError(String);

impl Band {
    /// Assign a band to a score using half-open intervals:
    /// `[0.0, 0.25)` → `Low`,
    /// `[0.25, 0.5)` → `MediumLow`,
    /// `[0.5, 0.75)` → `MediumHigh`,
    /// `[0.75, 1.0]` → `HighQuality`.
    pub fn from_score(score: Score) -> Self {
        let value: f32 = score.into();
        if value < 0.25 {
            Self::Low
        } else if value < 0.5 {
            Self::MediumLow
        } else if value < 0.75 {
            Self::MediumHigh
        } else {
            Self::HighQuality
        }
    }

    pub const ALL: &'static [Self] = &[Self::Low, Self::MediumLow, Self::MediumHigh, Self::HighQuality];

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::MediumLow => "MediumLow",
            Self::MediumHigh => "MediumHigh",
            Self::HighQuality => "HighQuality",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::MediumLow => "MedLow",
            Self::MediumHigh => "MedHigh",
            Self::HighQuality => "High",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Low => "Doesn't match the general layout at all; should be blocked at Prune stage",
            Self::MediumLow => "Technically matches the general layout but doesn't match the theme",
            Self::MediumHigh => "Matches the general layout and theme, but not great",
            Self::HighQuality => "Great exemplar of the original idea, or really interesting",
        }
    }

    /// Whether items in this band should appear in the public feed.
    pub const fn is_visible_in_feed(self) -> bool {
        matches!(self, Self::MediumHigh | Self::HighQuality)
    }
}

impl std::fmt::Display for Band {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.wire_name())
    }
}

impl std::str::FromStr for Band {
    type Err = ParseBandError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Low" => Ok(Self::Low),
            "MediumLow" => Ok(Self::MediumLow),
            "MediumHigh" => Ok(Self::MediumHigh),
            "HighQuality" => Ok(Self::HighQuality),
            other => Err(ParseBandError(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn score(value: f32) -> Score {
        Score::new(value).expect("valid score")
    }

    /// Documents the exact threshold values (0.25, 0.5, 0.75) and their boundary behaviour.
    #[test]
    fn from_score_boundaries() {
        assert_eq!(Band::from_score(score(0.0)), Band::Low);
        assert_eq!(Band::from_score(score(0.24)), Band::Low);
        assert_eq!(Band::from_score(score(0.25)), Band::MediumLow);
        assert_eq!(Band::from_score(score(0.49)), Band::MediumLow);
        assert_eq!(Band::from_score(score(0.5)), Band::MediumHigh);
        assert_eq!(Band::from_score(score(0.74)), Band::MediumHigh);
        assert_eq!(Band::from_score(score(0.75)), Band::HighQuality);
        assert_eq!(Band::from_score(score(1.0)), Band::HighQuality);
    }

    #[test]
    fn short_labels_are_distinct() {
        let labels: Vec<_> = Band::ALL.iter().map(|b| b.short_label()).collect();
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(labels.len(), unique.len());
    }

    #[test]
    fn descriptions_are_distinct() {
        let descs: Vec<_> = Band::ALL.iter().map(|b| b.description()).collect();
        let unique: std::collections::HashSet<_> = descs.iter().collect();
        assert_eq!(descs.len(), unique.len());
    }

    #[test]
    fn roundtrips_through_string() {
        for band in Band::ALL {
            let parsed: Band = band.to_string().parse().expect("roundtrip");
            assert_eq!(parsed, *band);
        }
    }

    #[test]
    fn rejects_unknown_band() {
        assert!("Nope".parse::<Band>().is_err());
        assert!("".parse::<Band>().is_err());
        assert!("low".parse::<Band>().is_err()); // case-sensitive
    }

    proptest! {
        /// `from_score` is non-decreasing: higher scores never yield lower bands.
        #[test]
        fn band_from_score_monotone(a in 0.0f32..=1.0f32, b in 0.0f32..=1.0f32) {
            let sa = Score::new(a).expect("valid");
            let sb = Score::new(b).expect("valid");
            if a <= b {
                prop_assert!(Band::from_score(sa) <= Band::from_score(sb));
            }
        }

        /// Visibility iff band ≥ MediumHigh — the threshold is always consistent.
        #[test]
        fn band_visibility_matches_threshold(a in 0.0f32..=1.0f32) {
            let band = Band::from_score(Score::new(a).expect("valid"));
            prop_assert_eq!(band.is_visible_in_feed(), band >= Band::MediumHigh);
        }
    }
}
