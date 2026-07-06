use serde::{Deserialize, Serialize};

use crate::skeet_id::SkeetId;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlocklistConfig {
    pub blocked: Vec<BlockedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedEntry {
    #[serde(rename = "at_uri")]
    pub skeet_id: SkeetId,
    pub reason: String,
}

/// Failure reading or writing a [`BlocklistConfig`] TOML file.
#[derive(Debug, thiserror::Error)]
pub enum BlocklistError {
    #[error("failed to read/write blocklist file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse blocklist TOML: {0}")]
    Deserialize(#[from] toml::de::Error),
    #[error("failed to serialize blocklist TOML: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl BlocklistConfig {
    /// Load blocklist configuration from a TOML file at the given path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, BlocklistError> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&text)?;
        config.sort();
        Ok(config)
    }

    /// Save the full blocklist to a TOML file at the given path.
    pub fn save(&self, path: &std::path::Path) -> Result<(), BlocklistError> {
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Check whether the given skeet is already in the blocklist.
    pub fn contains(&self, skeet_id: &SkeetId) -> bool {
        self.blocked.iter().any(|e| e.skeet_id == *skeet_id)
    }

    /// Add an entry to the blocklist, maintaining sorted order.
    /// Returns `false` if the skeet was already present.
    pub fn add(&mut self, entry: BlockedEntry) -> bool {
        if self.contains(&entry.skeet_id) {
            return false;
        }
        self.blocked.push(entry);
        self.sort();
        true
    }

    fn sort(&mut self) {
        self.blocked.sort_by(|a, b| a.skeet_id.cmp(&b.skeet_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(uri: &str) -> BlockedEntry {
        BlockedEntry {
            skeet_id: uri.parse().expect("valid AT URI"),
            reason: "test".to_string(),
        }
    }

    #[test]
    fn empty_blocklist_contains_nothing() {
        let config = BlocklistConfig::default();
        let id = "at://did:plc:abc/app.bsky.feed.post/xyz"
            .parse()
            .expect("valid");
        assert!(!config.contains(&id));
    }

    #[test]
    fn add_makes_entry_findable() {
        let mut config = BlocklistConfig::default();
        let uri = "at://did:plc:abc/app.bsky.feed.post/xyz";
        let id = uri.parse().expect("valid");
        assert!(config.add(entry(uri)));
        assert!(config.contains(&id));
    }

    #[test]
    fn add_returns_false_for_duplicate() {
        let mut config = BlocklistConfig::default();
        let uri = "at://did:plc:abc/app.bsky.feed.post/xyz";
        assert!(config.add(entry(uri)));
        assert!(!config.add(entry(uri)));
        assert_eq!(config.blocked.len(), 1);
    }

    #[test]
    fn add_maintains_sorted_order() {
        let mut config = BlocklistConfig::default();
        config.add(entry("at://did:plc:zzz/app.bsky.feed.post/rkey"));
        config.add(entry("at://did:plc:aaa/app.bsky.feed.post/rkey"));
        let ids: Vec<_> = config
            .blocked
            .iter()
            .map(|e| e.skeet_id.to_string())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }
}
