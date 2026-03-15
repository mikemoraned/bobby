use chrono::{DateTime, Utc};
use image::DynamicImage;
pub use shared::SkeetId;
use shared::ConfigVersion;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageId(Uuid);

impl ImageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

impl Default for ImageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ImageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for ImageId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredAt(DateTime<Utc>);

impl DiscoveredAt {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub const fn as_datetime(&self) -> &DateTime<Utc> {
        &self.0
    }

    pub const fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }
}

#[derive(Debug, Clone)]
pub struct OriginalAt(DateTime<Utc>);

impl OriginalAt {
    pub const fn new(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub const fn as_datetime(&self) -> &DateTime<Utc> {
        &self.0
    }

    pub const fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Archetype {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::TopLeft => "TOP_LEFT",
            Self::TopRight => "TOP_RIGHT",
            Self::BottomLeft => "BOTTOM_LEFT",
            Self::BottomRight => "BOTTOM_RIGHT",
        }
    }
}

impl std::str::FromStr for Archetype {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "TOP_LEFT" => Ok(Self::TopLeft),
            "TOP_RIGHT" => Ok(Self::TopRight),
            "BOTTOM_LEFT" => Ok(Self::BottomLeft),
            "BOTTOM_RIGHT" => Ok(Self::BottomRight),
            other => Err(format!("unknown archetype: {other}")),
        }
    }
}

impl std::fmt::Display for Archetype {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug)]
pub struct ImageRecord {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub image: DynamicImage,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
    pub archetype: Archetype,
    pub annotated_image: DynamicImage,
    pub config_version: ConfigVersion,
    pub detected_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_id_roundtrips_through_string() {
        let id = ImageId::new();
        let parsed: ImageId = id.as_str().parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn skeet_id_preserves_value() {
        let id = SkeetId::new("at://did:plc:abc123/app.bsky.feed.post/xyz");
        assert_eq!(id.as_str(), "at://did:plc:abc123/app.bsky.feed.post/xyz");
    }
}
