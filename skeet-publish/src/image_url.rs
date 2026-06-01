use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

/// A resolved, publicly fetchable image URL
///
/// Wraps a parsed [`url::Url`] so a constructed `ImageUrl` is always a valid,
/// absolute `https` url.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageUrl(Url);

#[derive(Debug, thiserror::Error)]
pub enum InvalidImageUrl {
    #[error("not a valid url: \"{input}\": {source}")]
    Unparseable {
        input: String,
        source: url::ParseError,
    },
    #[error("expected an https url, got scheme \"{scheme}\" in \"{input}\"")]
    NotHttps { input: String, scheme: String },
}

impl ImageUrl {
    pub fn new(s: impl AsRef<str>) -> Result<Self, InvalidImageUrl> {
        let s = s.as_ref();
        let url = Url::parse(s).map_err(|source| InvalidImageUrl::Unparseable {
            input: s.to_string(),
            source,
        })?;
        if url.scheme() != "https" {
            return Err(InvalidImageUrl::NotHttps {
                input: s.to_string(),
                scheme: url.scheme().to_string(),
            });
        }
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ImageUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for ImageUrl {
    type Err = InvalidImageUrl;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl Serialize for ImageUrl {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for ImageUrl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str =
        "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/bafyfakecid@jpeg";

    #[test]
    fn accepts_https() {
        assert_eq!(ImageUrl::new(SAMPLE).expect("valid").as_str(), SAMPLE);
    }

    #[test]
    fn rejects_non_https() {
        assert!(matches!(
            ImageUrl::new("http://insecure"),
            Err(InvalidImageUrl::NotHttps { .. })
        ));
    }

    #[test]
    fn rejects_unparseable() {
        assert!(matches!(
            ImageUrl::new("not a url"),
            Err(InvalidImageUrl::Unparseable { .. })
        ));
        assert!(ImageUrl::new("").is_err());
    }

    #[test]
    fn serde_roundtrips_through_json() {
        let url = ImageUrl::new(SAMPLE).expect("valid");
        let json = serde_json::to_string(&url).expect("serialize");
        assert_eq!(json, format!("\"{SAMPLE}\""));
        let parsed: ImageUrl = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, url);
    }

    #[test]
    fn deserialize_rejects_non_https() {
        assert!(serde_json::from_str::<ImageUrl>("\"http://insecure\"").is_err());
    }
}
