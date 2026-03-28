use std::fmt;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use serde::{Deserialize, Serialize};
use shared::ModelVersion;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelProvider(String);

impl ModelProvider {
    pub fn openai() -> Self {
        Self("openai".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelName(String);

impl ModelName {
    pub fn gpt_4o() -> Self {
        Self("gpt-4o".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoringPrompt(String);

impl ScoringPrompt {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self(prompt.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScoringPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringModel {
    pub model_provider: ModelProvider,
    pub model_name: ModelName,
    pub prompt: ScoringPrompt,
}

impl ScoringModel {
    pub fn version(&self) -> ModelVersion {
        let mut entries: Vec<(&str, &str)> = vec![
            ("model_name", self.model_name.as_str()),
            ("model_provider", self.model_provider.as_str()),
            ("prompt", self.prompt.as_str()),
        ];
        entries.sort_by_key(|(k, _)| *k);

        let mut hasher = DefaultHasher::new();
        for (k, v) in &entries {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
        let hash = hasher.finish();

        let mut version = String::with_capacity(8);
        write!(version, "{hash:016x}").expect("write to String");
        version.truncate(8);
        version.parse().expect("ModelVersion parse is infallible")
    }
}

pub fn load_model(path: &Path) -> Result<ScoringModel, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let model: ScoringModel = toml::from_str(&content)?;
    Ok(model)
}

pub fn save_model(path: &Path, model: &ScoringModel) -> Result<(), Box<dyn std::error::Error>> {
    let content = toml::to_string_pretty(model)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_deterministic() {
        let model = ScoringModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: ScoringPrompt::new("Rate this image"),
        };
        assert_eq!(model.version(), model.version());
    }

    #[test]
    fn version_changes_with_prompt() {
        let m1 = ScoringModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: ScoringPrompt::new("Rate this image"),
        };
        let m2 = ScoringModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: ScoringPrompt::new("Different prompt"),
        };
        assert_ne!(m1.version(), m2.version());
    }

    #[test]
    fn roundtrip_model() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("model.toml");

        let model = ScoringModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: ScoringPrompt::new("Rate this image"),
        };

        save_model(&path, &model).expect("save");
        let loaded = load_model(&path).expect("load");
        assert_eq!(loaded.model_provider, ModelProvider::openai());
        assert_eq!(loaded.model_name, ModelName::gpt_4o());
        assert_eq!(loaded.prompt, ScoringPrompt::new("Rate this image"));
    }
}
