#![warn(clippy::all, clippy::nursery)]

use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Utc};
use image::DynamicImage;
mod blocklist;
pub mod labels;
pub mod skeet_id;
pub mod tracing;

pub use blocklist::{BlockedEntry, BlocklistConfig};
use serde::Deserialize;
use skeet_id::SkeetId;

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
pub enum RejectionCategory {
    Face,
    Metadata,
}

impl std::fmt::Display for RejectionCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Face => write!(f, "Face"),
            Self::Metadata => write!(f, "Metadata"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rejection {
    FaceTooSmall,
    FaceTooLarge,
    FaceNotInAcceptedZone,
    TooManyFaces,
    TooFewFrontalFaces,
    TooLittleFaceSkin,
    TooMuchSkinOutsideFace,
    BlockedByMetadata,
}

impl Rejection {
    pub const fn category(self) -> RejectionCategory {
        match self {
            Self::FaceTooSmall
            | Self::FaceTooLarge
            | Self::FaceNotInAcceptedZone
            | Self::TooManyFaces
            | Self::TooFewFrontalFaces
            | Self::TooLittleFaceSkin
            | Self::TooMuchSkinOutsideFace => RejectionCategory::Face,
            Self::BlockedByMetadata => RejectionCategory::Metadata,
        }
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FaceTooSmall => write!(f, "FaceTooSmall"),
            Self::FaceTooLarge => write!(f, "FaceTooLarge"),
            Self::FaceNotInAcceptedZone => write!(f, "FaceNotInAcceptedZone"),
            Self::TooManyFaces => write!(f, "TooManyFaces"),
            Self::TooFewFrontalFaces => write!(f, "TooFewFrontalFaces"),
            Self::TooLittleFaceSkin => write!(f, "TooLittleFaceSkin"),
            Self::TooMuchSkinOutsideFace => write!(f, "TooMuchSkinOutsideFace"),
            Self::BlockedByMetadata => write!(f, "BlockedByMetadata"),
        }
    }
}

impl std::str::FromStr for Rejection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "FaceTooSmall" => Ok(Self::FaceTooSmall),
            "FaceTooLarge" => Ok(Self::FaceTooLarge),
            "FaceNotInAcceptedZone" => Ok(Self::FaceNotInAcceptedZone),
            "TooManyFaces" => Ok(Self::TooManyFaces),
            "TooFewFrontalFaces" => Ok(Self::TooFewFrontalFaces),
            "TooLittleFaceSkin" => Ok(Self::TooLittleFaceSkin),
            "TooMuchSkinOutsideFace" => Ok(Self::TooMuchSkinOutsideFace),
            "BlockedByMetadata" => Ok(Self::BlockedByMetadata),
            other => Err(format!("unknown rejection: {other}")),
        }
    }
}

/// A short hash string identifying a particular model or config version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelVersion(String);

impl ModelVersion {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for ModelVersion {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
    }
}

impl From<&str> for ModelVersion {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// A score in the range 0.0–1.0, where 1.0 is the best match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score(f32);

/// Quality band for appraising skeets and images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Low,
    MediumLow,
    MediumHigh,
    HighQuality,
}

impl Band {
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

    pub const fn is_visible_in_feed(self) -> bool {
        matches!(self, Self::MediumHigh | Self::HighQuality)
    }
}

impl std::fmt::Display for Band {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "Low"),
            Self::MediumLow => write!(f, "MediumLow"),
            Self::MediumHigh => write!(f, "MediumHigh"),
            Self::HighQuality => write!(f, "HighQuality"),
        }
    }
}

impl std::str::FromStr for Band {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Low" => Ok(Self::Low),
            "MediumLow" => Ok(Self::MediumLow),
            "MediumHigh" => Ok(Self::MediumHigh),
            "HighQuality" => Ok(Self::HighQuality),
            other => Err(format!("unknown band: {other}")),
        }
    }
}

impl PartialOrd for Band {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Band {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use Band::*;
        match (self, other) {
            (Low, Low) => std::cmp::Ordering::Equal,
            (Low, _) => std::cmp::Ordering::Less,
            (_, Low) => std::cmp::Ordering::Greater,
            (MediumLow, MediumLow) => std::cmp::Ordering::Equal,
            (MediumLow, _) => std::cmp::Ordering::Less,
            (_, MediumLow) => std::cmp::Ordering::Greater,
            (MediumHigh, MediumHigh) => std::cmp::Ordering::Equal,
            (MediumHigh, HighQuality) => std::cmp::Ordering::Less,
            (HighQuality, MediumHigh) => std::cmp::Ordering::Greater,
            (HighQuality, HighQuality) => std::cmp::Ordering::Equal,
        }
    }
}

/// Identity of whoever made an appraisal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Appraiser {
    GitHub { username: String },
}

impl Appraiser {
    pub fn wire_format(&self) -> String {
        match self {
            Self::GitHub { username } => format!("github:{username}"),
        }
    }

    pub fn from_wire_format(s: &str) -> Result<Self, String> {
        let Some((provider, identifier)) = s.split_once(':') else {
            return Err(format!("invalid appraiser format: {s}"));
        };
        match provider {
            "github" => Ok(Self::GitHub {
                username: identifier.to_string(),
            }),
            other => Err(format!("unknown appraiser provider: {other}")),
        }
    }
}

impl std::fmt::Display for Appraiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.wire_format())
    }
}

impl std::str::FromStr for Appraiser {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_wire_format(s)
    }
}

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

/// Configuration for prune classification thresholds.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PruneConfig {
    pub min_face_area_pct: Percentage,
    pub max_face_area_pct: Percentage,
    pub min_face_skin_pct: Percentage,
    pub max_outside_face_skin_pct: Percentage,
}

impl PruneConfig {
    /// Load configuration from a TOML file at the given path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&text)?;
        Ok(config)
    }

    /// Compute a version string by hashing all config values.
    ///
    /// The version is a short hex string derived from sorting all config
    /// key-value pairs and hashing them. Changing any threshold value
    /// produces a different version.
    pub fn version(&self) -> ModelVersion {
        let mut entries = vec![
            (
                "max_face_area_pct",
                self.max_face_area_pct.value().to_bits(),
            ),
            (
                "max_outside_face_skin_pct",
                self.max_outside_face_skin_pct.value().to_bits(),
            ),
            (
                "min_face_area_pct",
                self.min_face_area_pct.value().to_bits(),
            ),
            (
                "min_face_skin_pct",
                self.min_face_skin_pct.value().to_bits(),
            ),
        ];
        entries.sort_by_key(|(k, _)| *k);

        let mut hasher = DefaultHasher::new();
        for (k, v) in &entries {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        let hash = hasher.finish();

        let mut version = String::with_capacity(8);
        // Take first 8 hex chars for a short but unique-enough string
        write!(version, "{hash:016x}").expect("write to String");
        version.truncate(8);
        ModelVersion(version)
    }
}

/// Result of classifying an image: either an accepted zone or rejection reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    Accepted(Zone),
    Rejected(Vec<Rejection>),
}

/// A zone within the image, defined by overlaying a 4x4 grid and taking 2x2
/// blocks at each valid offset (0, 1, 2) in both X and Y, giving 9 zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    CenterCenter,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl std::fmt::Display for Zone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopLeft => write!(f, "TOP_LEFT"),
            Self::TopCenter => write!(f, "TOP_CENTER"),
            Self::TopRight => write!(f, "TOP_RIGHT"),
            Self::CenterLeft => write!(f, "CENTER_LEFT"),
            Self::CenterCenter => write!(f, "CENTER_CENTER"),
            Self::CenterRight => write!(f, "CENTER_RIGHT"),
            Self::BottomLeft => write!(f, "BOTTOM_LEFT"),
            Self::BottomCenter => write!(f, "BOTTOM_CENTER"),
            Self::BottomRight => write!(f, "BOTTOM_RIGHT"),
        }
    }
}

impl std::str::FromStr for Zone {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "TOP_LEFT" => Ok(Self::TopLeft),
            "TOP_CENTER" => Ok(Self::TopCenter),
            "TOP_RIGHT" => Ok(Self::TopRight),
            "CENTER_LEFT" => Ok(Self::CenterLeft),
            "CENTER_CENTER" => Ok(Self::CenterCenter),
            "CENTER_RIGHT" => Ok(Self::CenterRight),
            "BOTTOM_LEFT" => Ok(Self::BottomLeft),
            "BOTTOM_CENTER" => Ok(Self::BottomCenter),
            "BOTTOM_RIGHT" => Ok(Self::BottomRight),
            other => Err(format!("unknown zone: {other}")),
        }
    }
}

pub struct SkeetImage {
    pub skeet_id: SkeetId,
    pub original_at: DateTime<Utc>,
    pub image: DynamicImage,
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
    fn rejection_categories() {
        assert_eq!(Rejection::FaceTooSmall.category(), RejectionCategory::Face);
        assert_eq!(
            Rejection::BlockedByMetadata.category(),
            RejectionCategory::Metadata
        );
    }

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

    #[test]
    fn model_version_roundtrips_through_string() {
        let v = ModelVersion::from("abc123");
        let parsed: ModelVersion = v.to_string().parse().expect("should parse");
        assert_eq!(parsed, v);
    }

    #[test]
    fn zone_roundtrips_through_string() {
        for z in [
            Zone::TopLeft,
            Zone::TopCenter,
            Zone::TopRight,
            Zone::CenterLeft,
            Zone::CenterCenter,
            Zone::CenterRight,
            Zone::BottomLeft,
            Zone::BottomCenter,
            Zone::BottomRight,
        ] {
            let s = z.to_string();
            let parsed: Zone = s.parse().expect("should parse");
            assert_eq!(parsed, z);
        }
    }

    #[test]
    fn band_from_score_boundaries() {
        assert_eq!(Band::from_score(Score::new(0.0).unwrap()), Band::Low);
        assert_eq!(Band::from_score(Score::new(0.24).unwrap()), Band::Low);
        assert_eq!(Band::from_score(Score::new(0.25).unwrap()), Band::MediumLow);
        assert_eq!(Band::from_score(Score::new(0.49).unwrap()), Band::MediumLow);
        assert_eq!(Band::from_score(Score::new(0.5).unwrap()), Band::MediumHigh);
        assert_eq!(
            Band::from_score(Score::new(0.74).unwrap()),
            Band::MediumHigh
        );
        assert_eq!(
            Band::from_score(Score::new(0.75).unwrap()),
            Band::HighQuality
        );
        assert_eq!(
            Band::from_score(Score::new(1.0).unwrap()),
            Band::HighQuality
        );
    }

    #[test]
    fn band_is_visible_in_feed() {
        assert!(!Band::Low.is_visible_in_feed());
        assert!(!Band::MediumLow.is_visible_in_feed());
        assert!(Band::MediumHigh.is_visible_in_feed());
        assert!(Band::HighQuality.is_visible_in_feed());
    }

    #[test]
    fn band_display_and_fromstr() {
        for band in [
            Band::Low,
            Band::MediumLow,
            Band::MediumHigh,
            Band::HighQuality,
        ] {
            let s = band.to_string();
            let parsed: Band = s.parse().expect("should parse");
            assert_eq!(parsed, band);
        }
    }

    #[test]
    fn band_ordering() {
        assert!(Band::Low < Band::MediumLow);
        assert!(Band::MediumLow < Band::MediumHigh);
        assert!(Band::MediumHigh < Band::HighQuality);
        assert_eq!(
            Band::HighQuality.cmp(&Band::HighQuality),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn appraiser_display_and_fromstr() {
        let appraiser = Appraiser::GitHub {
            username: "testuser".to_string(),
        };
        let s = appraiser.to_string();
        assert_eq!(s, "github:testuser");
        let parsed: Appraiser = s.parse().expect("should parse");
        assert_eq!(parsed, appraiser);
    }

    #[test]
    fn appraiser_rejects_malformed() {
        assert!("invalid".parse::<Appraiser>().is_err());
        assert!("unknown:user".parse::<Appraiser>().is_err());
    }
}
