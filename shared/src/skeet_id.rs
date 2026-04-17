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
    use proptest::prelude::*;

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
    fn equal_skeet_ids() {
        let a: SkeetId = "at://did:plc:abc/app.bsky.feed.post/xyz".parse().unwrap();
        let b: SkeetId = "at://did:plc:abc/app.bsky.feed.post/xyz".parse().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn unequal_skeet_ids_differ_by_component() {
        let base: SkeetId = "at://did:plc:abc/app.bsky.feed.post/xyz".parse().unwrap();
        assert_ne!(base, "at://did:plc:zzz/app.bsky.feed.post/xyz".parse::<SkeetId>().unwrap());
        assert_ne!(base, "at://did:plc:abc/app.bsky.feed.get/xyz".parse::<SkeetId>().unwrap());
        assert_ne!(base, "at://did:plc:abc/app.bsky.feed.post/abc".parse::<SkeetId>().unwrap());
    }

    #[test]
    fn nsid_as_str_and_equality() {
        let id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/xyz".parse().unwrap();
        assert_eq!(id.collection().as_str(), "app.bsky.feed.post");
        // compare &Nsid != &str directly to exercise PartialEq<str> for Nsid
        assert_ne!(id.collection(), "app.bsky.feed.like");
    }

    proptest! {
        #[test]
        fn equal_skeet_ids_have_same_hash(
            did in "[a-z][a-z0-9:]{1,10}",
            collection in "[a-z][a-z0-9.]{1,10}",
            rkey in "[a-z][a-z0-9]{1,10}",
        ) {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let uri = format!("at://{did}/{collection}/{rkey}");
            let a: SkeetId = uri.parse().expect("valid");
            let b: SkeetId = uri.parse().expect("valid");
            let hash = |id: &SkeetId| { let mut h = DefaultHasher::new(); id.hash(&mut h); h.finish() };
            prop_assert_eq!(hash(&a), hash(&b));
        }

        #[test]
        fn different_skeet_ids_have_different_hashes(
            did_a in "[a-z][a-z0-9]{1,10}",
            did_b in "[a-z][a-z0-9]{1,10}",
        ) {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            prop_assume!(did_a != did_b);
            let a: SkeetId = format!("at://{did_a}/app.bsky.feed.post/rkey").parse().expect("valid");
            let b: SkeetId = format!("at://{did_b}/app.bsky.feed.post/rkey").parse().expect("valid");
            let hash = |id: &SkeetId| { let mut h = DefaultHasher::new(); id.hash(&mut h); h.finish() };
            prop_assert_ne!(hash(&a), hash(&b));
        }

        #[test]
        fn skeet_id_ordering_is_not_none(
            did_a in "[a-z][a-z0-9]{1,10}",
            did_b in "[a-z][a-z0-9]{1,10}",
        ) {
            let a: SkeetId = format!("at://{did_a}/app.bsky.feed.post/rkey").parse().expect("valid");
            let b: SkeetId = format!("at://{did_b}/app.bsky.feed.post/rkey").parse().expect("valid");
            prop_assert!(a.partial_cmp(&b).is_some());
            prop_assert_eq!(a.partial_cmp(&b), Some(did_a.cmp(&did_b)));
        }

        /// Arbitrary (did, collection, rkey) triples without '/' round-trip through
        /// the AT URI format: `at://{did}/{collection}/{rkey}`.
        #[test]
        fn skeet_id_roundtrip(
            did in "[a-z][a-z0-9:]{1,20}",
            collection in "[a-z][a-z0-9.]{1,20}",
            rkey in "[a-z][a-z0-9_-]{1,20}",
        ) {
            let uri = format!("at://{did}/{collection}/{rkey}");
            let id: SkeetId = uri.parse().expect("valid AT URI");
            prop_assert_eq!(id.to_string(), uri);
        }

        /// Strings without the `at://` prefix are always rejected.
        #[test]
        fn skeet_id_rejects_no_prefix(s in "[a-zA-Z0-9._-]{1,40}") {
            prop_assert!(s.parse::<SkeetId>().is_err());
        }
    }
}
