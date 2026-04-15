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

impl BlocklistConfig {
    /// Load blocklist configuration from a TOML file at the given path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&text)?;
        config.sort();
        Ok(config)
    }

    /// Save the full blocklist to a TOML file at the given path.
    pub fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
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
