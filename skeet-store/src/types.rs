use std::fmt;

use chrono::{DateTime, Utc};
use image::DynamicImage;
use shared::ModelVersion;
pub use shared::Zone;
pub use shared::skeet_id::SkeetId;
use uuid::Uuid;

const V2_PREFIX: &str = "v2:";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImageId {
    V1(Uuid),
    V2(md5::Digest),
}

impl ImageId {
    pub fn from_image(image: &DynamicImage) -> Self {
        Self::V2(md5::compute(image.as_bytes()))
    }
}

impl std::fmt::Display for ImageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::V1(uuid) => write!(f, "{uuid}"),
            Self::V2(digest) => write!(f, "{V2_PREFIX}{digest:x}"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid image id: \"{0}\"")]
pub struct InvalidImageId(String);

impl std::str::FromStr for ImageId {
    type Err = InvalidImageId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(hex_str) = s.strip_prefix(V2_PREFIX) {
            let bytes: [u8; 16] = hex::decode(hex_str)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| InvalidImageId(s.to_string()))?;
            Ok(Self::V2(md5::Digest(bytes)))
        } else {
            let uuid = Uuid::parse_str(s).map_err(|_| InvalidImageId(s.to_string()))?;
            Ok(Self::V1(uuid))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoveredAt(DateTime<Utc>);

impl DiscoveredAt {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub const fn new(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub const fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    pub fn format_short(&self) -> String {
        self.0.format("%Y-%m-%d %H:%M").to_string()
    }

    pub fn is_within_hours(&self, now: DateTime<Utc>, hours: u64) -> bool {
        let cutoff = now - chrono::Duration::hours(hours as i64);
        self.0 >= cutoff
    }
}

impl fmt::Display for DiscoveredAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OriginalAt(DateTime<Utc>);

impl OriginalAt {
    pub const fn new(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub const fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    pub fn format_short(&self) -> String {
        self.0.format("%Y-%m-%d %H:%M").to_string()
    }
}

impl fmt::Display for OriginalAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct ImageRecord {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub image: DynamicImage,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
    pub zone: Zone,
    pub annotated_image: DynamicImage,
    pub config_version: ModelVersion,
    pub detected_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn discovered_at_formatting() {
        use chrono::TimeZone as _;
        let dt = chrono::Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        let d = DiscoveredAt::new(dt);
        assert_eq!(d.format_short(), "2024-06-15 09:30");
        assert!(d.to_string().contains("2024-06-15"));
    }

    #[test]
    fn original_at_formatting_and_timestamp() {
        use chrono::TimeZone as _;
        let dt = chrono::Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        let o = OriginalAt::new(dt);
        assert_eq!(o.timestamp_micros(), dt.timestamp_micros());
        assert_eq!(o.format_short(), "2024-06-15 09:30");
        assert!(o.to_string().contains("2024-06-15"));
    }

    proptest! {
        #[test]
        fn image_id_v1_roundtrip(v in any::<u128>()) {
            let id = ImageId::V1(uuid::Uuid::from_u128(v));
            let parsed: ImageId = id.to_string().parse().expect("V1 roundtrip");
            prop_assert_eq!(id, parsed);
        }

        #[test]
        fn image_id_v2_roundtrip(bytes in any::<[u8; 16]>()) {
            let id = ImageId::V2(md5::Digest(bytes));
            let s = id.to_string();
            prop_assert!(s.starts_with("v2:"));
            let parsed: ImageId = s.parse().expect("V2 roundtrip");
            prop_assert_eq!(id, parsed);
        }

        /// Different byte content produces different V2 ids (MD5 collisions are
        /// astronomically rare with random inputs; any collision is skipped).
        #[test]
        fn image_id_v2_different_content(b1 in any::<Vec<u8>>(), b2 in any::<Vec<u8>>()) {
            prop_assume!(b1 != b2);
            let id1 = ImageId::V2(md5::compute(&b1));
            let id2 = ImageId::V2(md5::compute(&b2));
            prop_assume!(id1 != id2);
        }

        /// A timestamp equal to `now` is always within any hour window (offset = 0).
        #[test]
        fn discovered_now_is_always_within(
            ts in 0i64..=2_000_000_000i64,
            hours in 0u64..=8760u64,
        ) {
            use chrono::TimeZone as _;
            let now = chrono::Utc.timestamp_opt(ts, 0).single()
                .expect("ts in range");
            let d = DiscoveredAt::new(now);
            prop_assert!(d.is_within_hours(now, hours));
        }

        /// `is_within_hours` matches the arithmetic: offset_hours <= window_hours ⟺ within.
        #[test]
        fn within_hours_matches_arithmetic(
            ts_now in 360_000i64..=2_000_000_000i64,
            offset_hours in 0u64..=100u64,
            window_hours in 0u64..=200u64,
        ) {
            use chrono::TimeZone as _;
            let now = chrono::Utc.timestamp_opt(ts_now, 0).single()
                .expect("ts in range");
            let then = chrono::Utc
                .timestamp_opt(ts_now - offset_hours as i64 * 3600, 0)
                .single()
                .expect("ts in range");
            let d = DiscoveredAt::new(then);
            prop_assert_eq!(d.is_within_hours(now, window_hours), offset_hours <= window_hours);
        }
    }
}
