#![warn(clippy::all, clippy::nursery)]

use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Utc};
use image::DynamicImage;
pub mod appraiser;
pub mod band;
mod blocklist;
pub mod labels;
mod rejection;
pub mod score;
pub mod skeet_id;
pub mod tracing;
mod zone;

pub use appraiser::{Appraiser, ParseAppraiserError};
pub use band::{Band, ParseBandError};
pub use blocklist::{BlockedEntry, BlocklistConfig};
pub use rejection::{Rejection, RejectionCategory};
pub use score::{InvalidScore, Score};
use serde::Deserialize;
use skeet_id::SkeetId;
pub use zone::Zone;

/// A percentage value in the range 0.0–100.0.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
pub struct Percentage(f32);

#[derive(Debug, Clone, thiserror::Error)]
#[error("percentage must be between 0.0 and 100.0, got {0}")]
pub struct InvalidPercentage(f32);

impl Percentage {
    pub fn new(value: f32) -> Result<Self, InvalidPercentage> {
        if (0.0..=100.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidPercentage(value))
        }
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

impl From<&str> for ModelVersion {
    fn from(s: &str) -> Self {
        Self(s.to_string())
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
            ("max_face_area_pct", self.max_face_area_pct.value().to_bits()),
            ("max_outside_face_skin_pct", self.max_outside_face_skin_pct.value().to_bits()),
            ("min_face_area_pct", self.min_face_area_pct.value().to_bits()),
            ("min_face_skin_pct", self.min_face_skin_pct.value().to_bits()),
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

pub struct SkeetImage {
    pub skeet_id: SkeetId,
    pub original_at: DateTime<Utc>,
    pub image: DynamicImage,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn model_version_roundtrips_through_string() {
        let v = ModelVersion::from("abc123");
        let roundtripped = ModelVersion::from(v.to_string().as_str());
        assert_eq!(roundtripped, v);
    }

    fn make_config(a: u32, b: u32, c: u32, d: u32) -> PruneConfig {
        PruneConfig {
            min_face_area_pct: Percentage::new(a as f32).expect("valid"),
            max_face_area_pct: Percentage::new(b as f32).expect("valid"),
            min_face_skin_pct: Percentage::new(c as f32).expect("valid"),
            max_outside_face_skin_pct: Percentage::new(d as f32).expect("valid"),
        }
    }

    proptest! {
        #[test]
        fn percentage_validity(x in proptest::num::f32::ANY) {
            let result = Percentage::new(x);
            let expected_valid = (0.0..=100.0).contains(&x);
            prop_assert_eq!(result.is_ok(), expected_valid);
        }

        #[test]
        fn percentage_ordering(i in 0u32..=100u32, j in 0u32..=100u32) {
            let a = Percentage::new(i as f32).expect("valid");
            let b = Percentage::new(j as f32).expect("valid");
            prop_assert_eq!(a.partial_cmp(&b), (i as f32).partial_cmp(&(j as f32)));
        }

        /// Same config always produces the same version string (pure, deterministic).
        #[test]
        fn equal_configs_hash_equal(
            a in 0u32..=100u32, b in 0u32..=100u32,
            c in 0u32..=100u32, d in 0u32..=100u32,
        ) {
            let config1 = make_config(a, b, c, d);
            let config2 = make_config(a, b, c, d);
            prop_assert_eq!(config1.version(), config2.version());
        }

        /// Different configs produce different version strings (hash collisions are
        /// astronomically rare given DefaultHasher's 64-bit output and our small domain).
        #[test]
        fn different_configs_hash_differently(
            a1 in 0u32..=100u32, b1 in 0u32..=100u32,
            c1 in 0u32..=100u32, d1 in 0u32..=100u32,
            a2 in 0u32..=100u32, b2 in 0u32..=100u32,
            c2 in 0u32..=100u32, d2 in 0u32..=100u32,
        ) {
            prop_assume!((a1, b1, c1, d1) != (a2, b2, c2, d2));
            let config1 = make_config(a1, b1, c1, d1);
            let config2 = make_config(a2, b2, c2, d2);
            prop_assert_ne!(config1.version(), config2.version());
        }
    }
}
