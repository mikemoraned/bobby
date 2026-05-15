use std::fmt;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use eval::Threshold;
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
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn gpt_4o() -> Self {
        Self::new("gpt-4o")
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
pub struct RefinePrompt(String);

impl RefinePrompt {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self(prompt.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RefinePrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefineModel {
    pub model_provider: ModelProvider,
    pub model_name: ModelName,
    pub prompt: RefinePrompt,
    /// Score boundary at or above which a refined image is treated as positive.
    pub decision_threshold: Threshold,
}

impl RefineModel {
    pub fn version(&self) -> ModelVersion {
        let threshold_str = format!("{:?}", f64::from(self.decision_threshold));
        let mut entries: Vec<(&str, &str)> = vec![
            ("decision_threshold", threshold_str.as_str()),
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
        ModelVersion::from(version.as_str())
    }
}

pub fn load_model(path: &Path) -> Result<RefineModel, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let model: RefineModel = toml::from_str(&content)?;
    Ok(model)
}

pub fn save_model(path: &Path, model: &RefineModel) -> Result<(), Box<dyn std::error::Error>> {
    let content = toml::to_string_pretty(model)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn threshold(value: f64) -> Threshold {
        Threshold::new(value).expect("valid")
    }

    #[test]
    fn version_is_deterministic() {
        let model = RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("Rate this image"),
            decision_threshold: threshold(0.5),
        };
        assert_eq!(model.version(), model.version());
    }

    #[test]
    fn version_changes_with_prompt() {
        let m1 = RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("Rate this image"),
            decision_threshold: threshold(0.5),
        };
        let m2 = RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("Different prompt"),
            decision_threshold: threshold(0.5),
        };
        assert_ne!(m1.version(), m2.version());
    }

    #[test]
    fn version_changes_with_decision_threshold() {
        let m1 = RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("Rate this image"),
            decision_threshold: threshold(0.5),
        };
        let m2 = RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("Rate this image"),
            decision_threshold: threshold(0.6),
        };
        assert_ne!(m1.version(), m2.version());
    }

    #[test]
    fn roundtrip_model() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("model.toml");

        let model = RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new("Rate this image"),
            decision_threshold: threshold(0.6),
        };

        save_model(&path, &model).expect("save");
        let loaded = load_model(&path).expect("load");
        assert_eq!(loaded.model_provider, ModelProvider::openai());
        assert_eq!(loaded.model_name, ModelName::gpt_4o());
        assert_eq!(loaded.prompt, RefinePrompt::new("Rate this image"));
        assert_eq!(loaded.decision_threshold, threshold(0.6));
    }
}
