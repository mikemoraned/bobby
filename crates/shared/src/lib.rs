#![warn(clippy::all, clippy::nursery)]

use std::hash::{DefaultHasher, Hash, Hasher};

use chrono::{DateTime, Utc};
use image::DynamicImage;
pub mod appraiser;
pub mod band;
mod blocklist;
mod bluesky_cid;
mod image_id;
pub mod labels;
pub mod model_version;
pub mod query_plan;
pub mod refine_model;
mod rejection;
pub mod score;
pub mod skeet_id;
pub mod tracing;
mod zone;

pub use appraiser::{Appraiser, ParseAppraiserError};
pub use band::{Band, ParseBandError};
pub use blocklist::{BlockedEntry, BlocklistConfig};
pub use bluesky_cid::{BlueskyCid, InvalidBlueskyCid};
pub use image_id::{ImageId, InvalidImageId};
pub use model_version::{HashScheme, ModelVersion};
pub use refine_model::{
    Label, ModelName, ModelProvider, RefineModel, RefineModels, RefineModelsError, RefinePrompt,
};
pub use rejection::{Rejection, RejectionCategories, RejectionCategory};
pub use score::{
    InvalidNormalizedScore, InvalidScore, InvalidThreshold, NormalizedScore, Score, Threshold,
};
use serde::Deserialize;
use skeet_id::SkeetId;
pub use zone::{ParseZoneError, Zone};

/// A percentage value in the range 0.0–100.0.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(transparent)]
pub struct Percentage(f32);

#[derive(Debug, Clone, thiserror::Error)]
#[error("percentage must be between 0.0 and 100.0, got {0}")]
pub struct InvalidPercentage(f32);

#[derive(Debug, Clone, thiserror::Error)]
#[error("count {count} exceeds total {total}")]
pub struct CountExceedsTotal {
    pub count: u32,
    pub total: u32,
}

impl Percentage {
    /// Validating constructor for untrusted input.
    pub fn new(value: f32) -> Result<Self, InvalidPercentage> {
        if (0.0..=100.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidPercentage(value))
        }
    }

    /// `count` out of `total` as a percentage (`count / total * 100`), or 0% when
    /// `total == 0`. Errors if `count > total`, which would put the result above
    /// 100% and break the [0, 100] invariant.
    pub fn from_counts(count: u32, total: u32) -> Result<Self, CountExceedsTotal> {
        if count > total {
            return Err(CountExceedsTotal { count, total });
        }
        Ok(if total == 0 {
            Self(0.0)
        } else {
            Self((count as f32 / total as f32) * 100.0)
        })
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

/// Configuration for prune classification thresholds.
#[derive(Debug, Clone, Deserialize)]
pub struct PruneConfig {
    pub min_face_area_pct: Percentage,
    pub max_face_area_pct: Percentage,
    pub min_face_skin_pct: Percentage,
    pub max_outside_face_skin_pct: Percentage,
    pub max_text_area_pct: Percentage,
    #[serde(skip)]
    categories: RejectionCategories,
}

impl PruneConfig {
    /// Load configuration from a TOML file at the given path.
    /// If `categories` is `None`, the default set is used.
    pub fn from_file(
        path: &std::path::Path,
        categories: Option<RejectionCategories>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&text)?;
        config.categories = categories.unwrap_or_default();
        Ok(config)
    }

    pub fn is_category_enabled(&self, category: RejectionCategory) -> bool {
        self.categories.contains(&category)
    }

    pub const fn categories(&self) -> &RejectionCategories {
        &self.categories
    }

    /// Compute a version string by hashing all config values and enabled
    /// categories. Changing any threshold or the set of enabled categories
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
                "max_text_area_pct",
                self.max_text_area_pct.value().to_bits(),
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

        let mut cat_names: Vec<_> = self.categories.iter().map(ToString::to_string).collect();
        cat_names.sort();
        for name in &cat_names {
            name.hash(&mut hasher);
        }

        let hash = hasher.finish();

        let mut version = format!("{hash:016x}");
        version.truncate(8);
        ModelVersion::new(HashScheme::V1, version)
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
    pub cid: BlueskyCid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn make_config(a: u32, b: u32, c: u32, d: u32) -> PruneConfig {
        PruneConfig {
            min_face_area_pct: Percentage::new(a as f32).expect("valid"),
            max_face_area_pct: Percentage::new(b as f32).expect("valid"),
            min_face_skin_pct: Percentage::new(c as f32).expect("valid"),
            max_outside_face_skin_pct: Percentage::new(d as f32).expect("valid"),
            max_text_area_pct: Percentage::new(10.0).expect("valid"),
            categories: RejectionCategories::default(),
        }
    }

    #[test]
    fn percentage_display() {
        assert_eq!(Percentage::new(0.0).expect("valid").to_string(), "0.0%");
        assert_eq!(Percentage::new(50.0).expect("valid").to_string(), "50.0%");
        assert_eq!(Percentage::new(100.0).expect("valid").to_string(), "100.0%");
    }

    proptest! {
        #[test]
        fn percentage_equality(i in 0u32..=100u32, j in 0u32..=100u32) {
            let a = Percentage::new(i as f32).expect("valid");
            let b = Percentage::new(j as f32).expect("valid");
            prop_assert_eq!(a == b, i == j);
        }

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
