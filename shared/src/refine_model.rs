use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model_version::HashScheme;
use crate::{ModelVersion, Score, Threshold};

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

impl std::fmt::Display for ModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl std::fmt::Display for ModelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl std::fmt::Display for RefinePrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefineModel {
    pub model_provider: ModelProvider,
    pub model_name: ModelName,
    pub prompt: RefinePrompt,
    pub decision_threshold: Threshold,
}

impl RefineModel {
    /// The canonical `ModelVersion` for this model. Always V2 — newly
    /// constructed models always include `decision_threshold` in the hash.
    /// Use this for training output, live-refine tagging, and any other
    /// "what is this model's id" need.
    pub fn version(&self) -> ModelVersion {
        self.version_under(HashScheme::V2)
    }

    /// Compute the version under an explicit scheme. Used by the loader to
    /// verify legacy entries against their persisted hash; other callers
    /// should prefer `version()`.
    pub fn version_under(&self, scheme: HashScheme) -> ModelVersion {
        let threshold_str;
        let mut entries: HashMap<&str, &str> = HashMap::new();
        entries.insert("model_name", self.model_name.as_str());
        entries.insert("model_provider", self.model_provider.as_str());
        entries.insert("prompt", self.prompt.as_str());
        if scheme == HashScheme::V2 {
            threshold_str = format!("{:?}", f64::from(self.decision_threshold));
            entries.insert("decision_threshold", threshold_str.as_str());
        }
        ModelVersion::compute(scheme, entries)
    }

    /// Whether a score produced by this model is considered a positive match.
    pub fn is_positive(&self, score: Score) -> bool {
        Threshold::from(score) >= self.decision_threshold
    }
}

/// A label that references exactly one model by hash (e.g. `production`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Label(String);

impl Label {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn production() -> Self {
        Self::new("production")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RefineModelsError {
    #[error("model_version mismatch: stored {stored}, recomputed {recomputed}")]
    VersionMismatch {
        stored: ModelVersion,
        recomputed: ModelVersion,
    },
    #[error("label {label} references unknown model_version {version}")]
    UnknownLabelVersion {
        label: String,
        version: ModelVersion,
    },
    #[error("duplicate model_version {version} — each model must appear once")]
    DuplicateVersion { version: ModelVersion },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml deserialize error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
}

/// Persisted form of a single model entry in `refine.toml`. The
/// `hash_scheme` lives on the prefix of `model_version`, not as a separate
/// field.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedEntry {
    model_version: ModelVersion,
    model_provider: ModelProvider,
    model_name: ModelName,
    prompt: RefinePrompt,
    decision_threshold: Threshold,
}

impl TryFrom<PersistedEntry> for RefineModel {
    type Error = RefineModelsError;

    fn try_from(e: PersistedEntry) -> Result<Self, Self::Error> {
        let model = Self {
            model_provider: e.model_provider,
            model_name: e.model_name,
            prompt: e.prompt,
            decision_threshold: e.decision_threshold,
        };
        let recomputed = model.version_under(e.model_version.scheme());
        if recomputed != e.model_version {
            return Err(RefineModelsError::VersionMismatch {
                stored: e.model_version,
                recomputed,
            });
        }
        Ok(model)
    }
}

/// The full on-disk representation of `refine.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedModels {
    #[serde(default)]
    labels: HashMap<String, ModelVersion>,
    #[serde(default, rename = "models")]
    models: Vec<PersistedEntry>,
}

/// A registry of `RefineModel` entries keyed by their version hash, with
/// named labels (e.g. `production`) pointing at specific hashes.
#[derive(Debug, Clone, Default)]
pub struct RefineModels {
    models: HashMap<ModelVersion, RefineModel>,
    labels: HashMap<Label, ModelVersion>,
}

impl RefineModels {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from a TOML file.  Returns an error if any entry's stored hash
    /// does not match the hash recomputed from its fields.
    pub fn load(path: &Path) -> Result<Self, RefineModelsError> {
        let text = std::fs::read_to_string(path)?;
        let persisted: PersistedModels = toml::from_str(&text)?;
        Self::from_persisted(persisted)
    }

    /// Load from a TOML file, or return an empty registry if the file does
    /// not exist.
    pub fn load_or_empty(path: &Path) -> Result<Self, RefineModelsError> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::new())
        }
    }

    fn from_persisted(persisted: PersistedModels) -> Result<Self, RefineModelsError> {
        let mut models: HashMap<ModelVersion, RefineModel> = HashMap::new();
        for entry in persisted.models {
            let version = entry.model_version.clone();
            let model = RefineModel::try_from(entry)?;
            if models.contains_key(&version) {
                return Err(RefineModelsError::DuplicateVersion { version });
            }
            models.insert(version, model);
        }

        let mut labels: HashMap<Label, ModelVersion> = HashMap::new();
        for (label_str, version) in persisted.labels {
            if !models.contains_key(&version) {
                return Err(RefineModelsError::UnknownLabelVersion {
                    label: label_str,
                    version,
                });
            }
            labels.insert(Label(label_str), version);
        }

        Ok(Self { models, labels })
    }

    /// Persist to a TOML file.
    pub fn save(&self, path: &Path) -> Result<(), RefineModelsError> {
        let mut persisted_labels: HashMap<String, ModelVersion> = HashMap::new();
        for (label, version) in &self.labels {
            persisted_labels.insert(label.0.clone(), version.clone());
        }

        let mut entries: Vec<PersistedEntry> = self
            .models
            .iter()
            .map(|(version, model)| PersistedEntry {
                model_version: version.clone(),
                model_provider: model.model_provider.clone(),
                model_name: model.model_name.clone(),
                prompt: model.prompt.clone(),
                decision_threshold: model.decision_threshold,
            })
            .collect();
        // Stable output: sort by serialised model_version.
        entries.sort_by_key(|e| e.model_version.to_string());

        let persisted = PersistedModels {
            labels: persisted_labels,
            models: entries,
        };
        let text = toml::to_string_pretty(&persisted)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Look up a model by its version hash.
    pub fn get(&self, version: &ModelVersion) -> Option<&RefineModel> {
        self.models.get(version)
    }

    /// Every registered `ModelVersion` — the "known set".
    pub fn versions(&self) -> impl Iterator<Item = &ModelVersion> {
        self.models.keys()
    }

    /// Look up a model by label (e.g. `Label::production()`).
    pub fn by_label(&self, label: &Label) -> Option<&RefineModel> {
        let version = self.labels.get(label)?;
        self.models.get(version)
    }

    /// Every label and the version it currently points at.
    pub fn labels(&self) -> impl Iterator<Item = (&Label, &ModelVersion)> {
        self.labels.iter()
    }

    /// Point `label` at an already-registered `version` (e.g. promotion:
    /// repoint `production`). No data migration — only the label moves. Errors
    /// with [`RefineModelsError::UnknownLabelVersion`] if `version` is not in
    /// the registry, mirroring the load-time validation.
    pub fn set_label(
        &mut self,
        label: Label,
        version: ModelVersion,
    ) -> Result<(), RefineModelsError> {
        if !self.models.contains_key(&version) {
            return Err(RefineModelsError::UnknownLabelVersion {
                label: label.to_string(),
                version,
            });
        }
        self.labels.insert(label, version);
        Ok(())
    }

    /// Register a model under its `ModelVersion`. Labels are managed separately
    /// via [`Self::set_label`] — registering a model never moves a label.
    /// Historical V1 entries are loaded via `load`, not `insert` — V1 is only a
    /// parse-time concept for legacy stored data.
    pub fn insert(&mut self, model: RefineModel) {
        self.models.insert(model.version(), model);
    }
}

/// Test-only helpers.
#[cfg(any(test, feature = "test-helpers"))]
impl RefineModels {
    /// Insert a model keyed by an arbitrary version string, bypassing hash
    /// verification.  Used in tests that seed scores with synthetic version
    /// strings (e.g. `"test"`) that are not real hash outputs.
    pub fn insert_unverified(&mut self, version_str: &str, model: RefineModel) {
        self.models.insert(ModelVersion::from(version_str), model);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn threshold(value: f64) -> Threshold {
        Threshold::new(value).expect("valid")
    }

    fn model_with(prompt: &str, thr: f64) -> RefineModel {
        RefineModel {
            model_provider: ModelProvider::openai(),
            model_name: ModelName::gpt_4o(),
            prompt: RefinePrompt::new(prompt),
            decision_threshold: threshold(thr),
        }
    }

    #[test]
    fn v1_version_ignores_decision_threshold() {
        let m1 = model_with("prompt", 0.5);
        let m2 = model_with("prompt", 0.7);
        assert_eq!(
            m1.version_under(HashScheme::V1),
            m2.version_under(HashScheme::V1)
        );
    }

    #[test]
    fn version_defaults_to_v2_and_includes_decision_threshold() {
        let m1 = model_with("prompt", 0.5);
        let m2 = model_with("prompt", 0.6);
        assert_eq!(m1.version().scheme(), HashScheme::V2);
        assert_ne!(m1.version(), m2.version());
    }

    #[test]
    fn is_positive_at_boundary() {
        let model = model_with("p", 0.5);
        let score_below = Score::new(0.499).expect("valid");
        let score_at = Score::new(0.5).expect("valid");
        let score_above = Score::new(0.6).expect("valid");
        assert!(!model.is_positive(score_below));
        assert!(model.is_positive(score_at));
        assert!(model.is_positive(score_above));
    }

    #[test]
    fn roundtrip_via_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("refine.toml");

        let mut models = RefineModels::new();
        models.insert(model_with("old prompt", 0.5));
        let new = model_with("new prompt", 0.6);
        let new_version = new.version();
        models.insert(new);
        models
            .set_label(Label::production(), new_version)
            .expect("registered");
        models.save(&path).expect("save");

        let loaded = RefineModels::load(&path).expect("load");
        let production = loaded
            .by_label(&Label::production())
            .expect("production resolves");
        assert_eq!(production.prompt.as_str(), "new prompt");
    }

    #[test]
    fn load_preserves_v1_and_v2_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("refine.toml");

        // Build a TOML with one V1 and one V2 entry by computing real hashes.
        let m_v1 = model_with("legacy prompt", 0.5);
        let v1_mv = m_v1.version_under(HashScheme::V1);
        let m_v2 = model_with("modern prompt", 0.5);
        let v2_mv = m_v2.version();

        let content = format!(
            r#"
[labels]

[[models]]
model_version = "{v1_mv}"
model_provider = "openai"
model_name = "gpt-4o"
decision_threshold = 0.5
prompt = "legacy prompt"

[[models]]
model_version = "{v2_mv}"
model_provider = "openai"
model_name = "gpt-4o"
decision_threshold = 0.5
prompt = "modern prompt"
"#,
        );
        std::fs::write(&path, content).expect("write");

        let loaded = RefineModels::load(&path).expect("load");
        assert!(!v1_mv.to_string().starts_with("v2:"));
        assert!(v2_mv.to_string().starts_with("v2:"));
        assert_eq!(
            loaded.get(&v1_mv).map(|m| m.prompt.as_str()),
            Some("legacy prompt")
        );
        assert_eq!(
            loaded.get(&v2_mv).map(|m| m.prompt.as_str()),
            Some("modern prompt")
        );
    }

    #[test]
    fn load_error_version_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("refine.toml");
        std::fs::write(
            &path,
            r#"
[labels]

[[models]]
model_version = "v2:deadbeef"
model_provider = "openai"
model_name = "gpt-4o"
decision_threshold = 0.5
prompt = "some prompt"
"#,
        )
        .expect("write");
        let err = RefineModels::load(&path).expect_err("should fail");
        assert!(matches!(err, RefineModelsError::VersionMismatch { .. }));
    }

    #[test]
    fn load_error_unknown_label_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("refine.toml");
        let model = model_with("some prompt", 0.5);
        let real = model.version();
        let content = format!(
            r#"
[labels]
production = "v2:ffffffff"

[[models]]
model_version = "{real}"
model_provider = "openai"
model_name = "gpt-4o"
decision_threshold = 0.5
prompt = "some prompt"
"#
        );
        std::fs::write(&path, content).expect("write");
        let err = RefineModels::load(&path).expect_err("should fail");
        assert!(matches!(err, RefineModelsError::UnknownLabelVersion { .. }));
    }

    #[test]
    fn load_error_duplicate_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("refine.toml");
        let model = model_with("prompt", 0.5);
        let version = model.version();
        let content = format!(
            r#"
[labels]

[[models]]
model_version = "{version}"
model_provider = "openai"
model_name = "gpt-4o"
decision_threshold = 0.5
prompt = "prompt"

[[models]]
model_version = "{version}"
model_provider = "openai"
model_name = "gpt-4o"
decision_threshold = 0.5
prompt = "prompt"
"#
        );
        std::fs::write(&path, content).expect("write");
        let err = RefineModels::load(&path).expect_err("should fail");
        assert!(matches!(err, RefineModelsError::DuplicateVersion { .. }));
    }

    #[test]
    fn set_label_repoints_to_registered_version() {
        let old = model_with("old prompt", 0.5);
        let new = model_with("new prompt", 0.6);
        let new_version = new.version();

        let old_version = old.version();
        let mut models = RefineModels::new();
        models.insert(old);
        models.insert(new);
        models
            .set_label(Label::production(), old_version)
            .expect("registered version");

        models
            .set_label(Label::production(), new_version)
            .expect("registered version");
        assert_eq!(
            models
                .by_label(&Label::production())
                .map(|m| m.prompt.as_str()),
            Some("new prompt")
        );
    }

    #[test]
    fn set_label_rejects_unregistered_version() {
        let mut models = RefineModels::new();
        models.insert(model_with("p", 0.5));
        let err = models
            .set_label(Label::production(), ModelVersion::from("v2:ffffffff"))
            .expect_err("unregistered");
        assert!(matches!(err, RefineModelsError::UnknownLabelVersion { .. }));
    }

    #[test]
    fn insert_leaves_labels_untouched() {
        let old = model_with("old prompt", 0.5);
        let new = model_with("new prompt", 0.6);
        let old_version = old.version();

        let mut models = RefineModels::new();
        models.insert(old);
        models
            .set_label(Label::production(), old_version.clone())
            .expect("registered");

        // Registering another model does not move the label off `old`.
        models.insert(new);
        assert_eq!(
            models
                .by_label(&Label::production())
                .map(|m| m.prompt.as_str()),
            Some("old prompt")
        );
        assert!(models.get(&old_version).is_some());
    }
}
