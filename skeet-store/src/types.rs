use chrono::{DateTime, Utc};
use image::DynamicImage;
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkeetId(String);

impl SkeetId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SkeetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredAt(DateTime<Utc>);

impl DiscoveredAt {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub fn as_datetime(&self) -> &DateTime<Utc> {
        &self.0
    }

    pub fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }
}

#[derive(Debug, Clone)]
pub struct OriginalAt(DateTime<Utc>);

impl OriginalAt {
    pub fn new(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub fn as_datetime(&self) -> &DateTime<Utc> {
        &self.0
    }

    pub fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }
}

#[derive(Debug)]
pub struct ImageRecord {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub image: DynamicImage,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
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
