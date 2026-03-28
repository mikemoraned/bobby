use std::fmt;

use chrono::{DateTime, Utc};
use image::DynamicImage;
use shared::ConfigVersion;
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
    pub config_version: ConfigVersion,
    pub detected_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_roundtrips_through_string() {
        let img = image::DynamicImage::new_rgba8(2, 2);
        let id = ImageId::from_image(&img);
        let s = id.to_string();
        assert!(s.starts_with("v2:"));
        let parsed: ImageId = s.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn v2_is_deterministic_for_same_content() {
        let img = image::DynamicImage::new_rgba8(2, 2);
        let id1 = ImageId::from_image(&img);
        let id2 = ImageId::from_image(&img);
        assert_eq!(id1, id2);
    }

    #[test]
    fn v2_differs_for_different_content() {
        let img1 = image::DynamicImage::new_rgba8(2, 2);
        let img2 = image::DynamicImage::new_rgba8(3, 3);
        assert_ne!(ImageId::from_image(&img1), ImageId::from_image(&img2));
    }

    #[test]
    fn v1_uuid_parsed_from_string() {
        let id: ImageId = "24950d63-d0b5-46c9-ac10-e4338362bd4c".parse().unwrap();
        assert!(matches!(id, ImageId::V1(_)));
        assert_eq!(id.to_string(), "24950d63-d0b5-46c9-ac10-e4338362bd4c");
    }

    #[test]
    fn v1_uuid_roundtrips_through_string() {
        let id: ImageId = "24950d63-d0b5-46c9-ac10-e4338362bd4c".parse().unwrap();
        let parsed: ImageId = id.to_string().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn v1_and_v2_are_not_equal() {
        let v1: ImageId = "24950d63-d0b5-46c9-ac10-e4338362bd4c".parse().unwrap();
        let img = image::DynamicImage::new_rgba8(2, 2);
        let v2 = ImageId::from_image(&img);
        assert_ne!(v1, v2);
    }

    #[test]
    fn rejects_invalid_string() {
        assert!("not-valid".parse::<ImageId>().is_err());
    }

    #[test]
    fn rejects_v2_with_bad_hex() {
        assert!("v2:not-hex".parse::<ImageId>().is_err());
    }

    #[test]
    fn skeet_id_preserves_value() {
        let id: SkeetId = "at://did:plc:abc123/app.bsky.feed.post/xyz"
            .parse()
            .expect("valid AT URI");
        assert_eq!(id.to_string(), "at://did:plc:abc123/app.bsky.feed.post/xyz");
    }
}
