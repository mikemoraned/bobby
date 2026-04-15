use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
#[error("invalid AT URI: {0}")]
pub struct SkeetIdError(String);

#[derive(Debug, Clone)]
pub struct SkeetId {
    did: Did,
    collection: Nsid,
    rkey: RecordKey,
}

impl SkeetId {
    pub fn new(id: impl Into<String>) -> Result<Self, SkeetIdError> {
        let s = id.into();
        let stripped = s.strip_prefix("at://").ok_or_else(|| SkeetIdError(s.clone()))?;
        let (did, rest) = stripped.split_once('/').ok_or_else(|| SkeetIdError(s.clone()))?;
        let (collection, rkey) = rest.split_once('/').ok_or_else(|| SkeetIdError(s.clone()))?;
        Ok(Self {
            did: Did(did.to_string()),
            collection: Nsid(collection.to_string()),
            rkey: RecordKey(rkey.to_string()),
        })
    }

    /// Construct a SkeetId for a Bluesky post from its DID and rkey.
    pub fn for_post(did: &str, rkey: &str) -> Self {
        Self {
            did: Did(did.to_string()),
            collection: Nsid("app.bsky.feed.post".to_string()),
            rkey: RecordKey(rkey.to_string()),
        }
    }

    pub const fn did(&self) -> &Did {
        &self.did
    }

    pub const fn collection(&self) -> &Nsid {
        &self.collection
    }

    pub const fn rkey(&self) -> &RecordKey {
        &self.rkey
    }
}

impl PartialEq for SkeetId {
    fn eq(&self, other: &Self) -> bool {
        self.did == other.did
            && self.collection == other.collection
            && self.rkey == other.rkey
    }
}

impl Eq for SkeetId {}

impl std::hash::Hash for SkeetId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.did.0.hash(state);
        self.collection.0.hash(state);
        self.rkey.0.hash(state);
    }
}

impl Ord for SkeetId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.did.0.cmp(&other.did.0)
            .then(self.collection.0.cmp(&other.collection.0))
            .then(self.rkey.0.cmp(&other.rkey.0))
    }
}

impl PartialOrd for SkeetId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::str::FromStr for SkeetId {
    type Err = SkeetIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl std::fmt::Display for SkeetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at://{}/{}/{}", self.did, self.collection, self.rkey)
    }
}

impl Serialize for SkeetId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SkeetId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// A Decentralized Identifier (DID), e.g. `did:plc:abc123`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Did(String);

impl Did {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Did {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A Namespaced Identifier (NSID) for AT Protocol collections, e.g. `app.bsky.feed.post`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nsid(String);

impl Nsid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Nsid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<&str> for Nsid {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<str> for Nsid {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

/// A record key (rkey) identifying a specific record within an AT Protocol collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKey(String);

impl RecordKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RecordKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_post_constructs_at_uri() {
        let id = SkeetId::for_post("did:plc:abc123", "xyz789");
        assert_eq!(id.to_string(), "at://did:plc:abc123/app.bsky.feed.post/xyz789");
    }

    #[test]
    fn extracts_components() {
        let id: SkeetId = "at://did:plc:abc123/app.bsky.feed.post/xyz789"
            .parse()
            .expect("valid AT URI");
        assert_eq!(id.did().as_str(), "did:plc:abc123");
        assert_eq!(id.collection(), "app.bsky.feed.post");
        assert_eq!(id.rkey().as_str(), "xyz789");
    }

    #[test]
    fn rejects_invalid_uri() {
        assert!("not-an-at-uri".parse::<SkeetId>().is_err());
    }
}
