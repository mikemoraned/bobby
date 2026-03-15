use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlocklistConfig {
    pub blocked: Vec<BlockedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedEntry {
    pub at_uri: String,
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

    /// Check whether the given at:// URI is already in the blocklist.
    pub fn contains(&self, at_uri: &str) -> bool {
        self.blocked.iter().any(|e| e.at_uri == at_uri)
    }

    /// Add an entry to the blocklist, maintaining sorted order by `at_uri`.
    /// Returns `false` if the URI was already present.
    pub fn add(&mut self, entry: BlockedEntry) -> bool {
        if self.contains(&entry.at_uri) {
            return false;
        }
        self.blocked.push(entry);
        self.sort();
        true
    }

    fn sort(&mut self) {
        self.blocked.sort_by(|a, b| a.at_uri.cmp(&b.at_uri));
    }
}
