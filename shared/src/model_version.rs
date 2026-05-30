use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The hashing algorithm used to derive a `ModelVersion`'s digits.
///
/// `V1` covers the original scheme that hashes only
/// `(model_provider, model_name, prompt)`. `V2` extends V1 by including
/// `decision_threshold` in the hash.
///
/// The scheme is encoded as a prefix on the serialised form of `ModelVersion`:
/// no prefix → V1, `v2:` prefix → V2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashScheme {
    V1,
    V2,
}

impl HashScheme {
    const V2_PREFIX: &'static str = "v2:";

    pub const fn prefix(self) -> &'static str {
        match self {
            Self::V1 => "",
            Self::V2 => Self::V2_PREFIX,
        }
    }
}

/// A versioned identifier for a refine model, combining a `HashScheme`
/// (which selects the hash algorithm) with the resulting hash digits.
///
/// Persisted as a single string: V1 entries appear as bare digits
/// (`"ea219ee0"`) for backward compatibility with pre-scheme storage; V2
/// entries are prefixed (`"v2:34d8bec0"`). The prefix is the sole signal of
/// scheme — no companion field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelVersion {
    scheme: HashScheme,
    hash: String,
}

impl ModelVersion {
    pub fn new(scheme: HashScheme, hash: impl Into<String>) -> Self {
        Self {
            scheme,
            hash: hash.into(),
        }
    }

    /// Compute a `ModelVersion` by hashing the given (key, value) entries
    /// under the named scheme.
    pub fn compute(scheme: HashScheme, entries: HashMap<&str, &str>) -> Self {
        let mut sorted: Vec<(&str, &str)> = entries.into_iter().collect();
        sorted.sort_by_key(|(k, _)| *k);

        let mut hasher = DefaultHasher::new();
        for (k, v) in &sorted {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        let raw = hasher.finish();

        let mut digits = String::with_capacity(8);
        write!(digits, "{raw:016x}").expect("write to String");
        digits.truncate(8);
        Self::new(scheme, digits)
    }

    pub const fn scheme(&self) -> HashScheme {
        self.scheme
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }
}

impl std::fmt::Display for ModelVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.scheme.prefix(), self.hash)
    }
}

impl From<&str> for ModelVersion {
    fn from(s: &str) -> Self {
        s.strip_prefix(HashScheme::V2_PREFIX).map_or_else(
            || Self {
                scheme: HashScheme::V1,
                hash: s.to_string(),
            },
            |hash| Self {
                scheme: HashScheme::V2,
                hash: hash.to_string(),
            },
        )
    }
}

impl Serialize for ModelVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ModelVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from(s.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_string_as_v1() {
        let v = ModelVersion::from("ea219ee0");
        assert_eq!(v.scheme(), HashScheme::V1);
        assert_eq!(v.hash(), "ea219ee0");
    }

    #[test]
    fn parses_v2_prefixed_string() {
        let v = ModelVersion::from("v2:34d8bec0");
        assert_eq!(v.scheme(), HashScheme::V2);
        assert_eq!(v.hash(), "34d8bec0");
    }

    #[test]
    fn display_v1_omits_prefix() {
        let v = ModelVersion::new(HashScheme::V1, "ea219ee0");
        assert_eq!(v.to_string(), "ea219ee0");
    }

    #[test]
    fn display_v2_adds_prefix() {
        let v = ModelVersion::new(HashScheme::V2, "34d8bec0");
        assert_eq!(v.to_string(), "v2:34d8bec0");
    }

    #[test]
    fn roundtrips_v1_through_string() {
        let original = ModelVersion::new(HashScheme::V1, "ea219ee0");
        let roundtripped = ModelVersion::from(original.to_string().as_str());
        assert_eq!(roundtripped, original);
    }

    #[test]
    fn roundtrips_v2_through_string() {
        let original = ModelVersion::new(HashScheme::V2, "34d8bec0");
        let roundtripped = ModelVersion::from(original.to_string().as_str());
        assert_eq!(roundtripped, original);
    }

    #[test]
    fn v1_and_v2_with_same_hash_are_distinct() {
        let v1 = ModelVersion::new(HashScheme::V1, "abc123");
        let v2 = ModelVersion::new(HashScheme::V2, "abc123");
        assert_ne!(v1, v2);
        assert_ne!(v1.to_string(), v2.to_string());
    }
}
