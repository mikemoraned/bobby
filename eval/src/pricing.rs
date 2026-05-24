use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, ParseError, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use shared::refine_model::Label;

use crate::usd::Usd;

const PRICES_TOML: &str = include_str!("../prices.toml");

#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    #[error("unknown model: {0}")]
    UnknownModel(String),
    #[error("failed to read/write {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse prices.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize prices.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("label {label} references unknown snapshot_id {snapshot_id}")]
    UnknownLabelSnapshot {
        label: String,
        snapshot_id: SnapshotId,
    },
    #[error("duplicate snapshot_id {0} — each snapshot must appear once")]
    DuplicateSnapshotId(SnapshotId),
    #[error("snapshot_id {0} not found in registry")]
    UnknownSnapshotId(SnapshotId),
    #[error("label {0} not found in registry")]
    UnknownLabel(Label),
}

/// Identifier for a pricing snapshot. The id *is* the UTC timestamp at which
/// the snapshot was fetched.
/// Serialised as an RFC3339 string (e.g. `2026-05-10T17:06:34.843738Z`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(DateTime<Utc>);

#[derive(Debug, thiserror::Error)]
#[error("invalid snapshot_id (expected RFC3339 UTC datetime): {input} ({source})")]
pub struct InvalidSnapshotId {
    input: String,
    #[source]
    source: ParseError,
}

impl SnapshotId {
    pub fn new(t: DateTime<Utc>) -> Self {
        Self(t)
    }

    pub fn fetched_at(&self) -> DateTime<Utc> {
        self.0
    }
}

impl FromStr for SnapshotId {
    type Err = InvalidSnapshotId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DateTime::parse_from_rfc3339(s)
            .map(|t| Self(t.with_timezone(&Utc)))
            .map_err(|e| InvalidSnapshotId {
                input: s.to_string(),
                source: e,
            })
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }
}

/// Per-million-token pricing for one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPrice {
    #[serde(rename = "input_per_million_usd")]
    pub input_per_million: Usd,
    #[serde(rename = "output_per_million_usd")]
    pub output_per_million: Usd,
}

/// One pinned set of per-model pricing, with provenance. Persisted as a
/// `[[snapshots]]` entry inside `PricesRegistry`. The capture time lives on
/// the `SnapshotId` rather than as a separate field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub prices: BTreeMap<String, ModelPrice>,
}

impl Snapshot {
    /// Dollar cost for `(input_tokens, output_tokens)` of `model_name` under
    /// this snapshot's prices. Errors if `model_name` is not present.
    pub fn cost_for(
        &self,
        model_name: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<Usd, PricingError> {
        let p = self
            .prices
            .get(model_name)
            .ok_or_else(|| PricingError::UnknownModel(model_name.to_string()))?;
        Ok(p.input_per_million * input_tokens / 1_000_000
            + p.output_per_million * output_tokens / 1_000_000)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSnapshot {
    snapshot_id: SnapshotId,
    source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    prices: BTreeMap<String, ModelPrice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedRegistry {
    #[serde(default)]
    labels: BTreeMap<String, SnapshotId>,
    #[serde(default, rename = "snapshots")]
    snapshots: Vec<PersistedSnapshot>,
}

/// Registry of pricing `Snapshot`s keyed by `SnapshotId`, with named labels
/// (e.g. `current`) pointing at canonical snapshots.
#[derive(Debug, Clone, Default)]
pub struct PricesRegistry {
    snapshots: BTreeMap<SnapshotId, Snapshot>,
    labels: HashMap<Label, SnapshotId>,
}

impl PricesRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the registry baked into the binary from `eval/prices.toml`.
    pub fn embedded() -> Result<Self, PricingError> {
        Self::from_toml_str(PRICES_TOML)
    }

    pub fn from_toml_str(s: &str) -> Result<Self, PricingError> {
        let persisted: PersistedRegistry = toml::from_str(s)?;
        Self::from_persisted(persisted)
    }

    pub fn load(path: &Path) -> Result<Self, PricingError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| PricingError::Io(path.display().to_string(), e))?;
        Self::from_toml_str(&text)
    }

    pub fn load_or_empty(path: &Path) -> Result<Self, PricingError> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::new())
        }
    }

    fn from_persisted(persisted: PersistedRegistry) -> Result<Self, PricingError> {
        let mut snapshots: BTreeMap<SnapshotId, Snapshot> = BTreeMap::new();
        for entry in persisted.snapshots {
            let id = entry.snapshot_id;
            if snapshots.contains_key(&id) {
                return Err(PricingError::DuplicateSnapshotId(id));
            }
            snapshots.insert(
                id,
                Snapshot {
                    source_url: entry.source_url,
                    note: entry.note,
                    prices: entry.prices,
                },
            );
        }

        let mut labels: HashMap<Label, SnapshotId> = HashMap::new();
        for (label_str, snapshot_id) in persisted.labels {
            if !snapshots.contains_key(&snapshot_id) {
                return Err(PricingError::UnknownLabelSnapshot {
                    label: label_str,
                    snapshot_id,
                });
            }
            labels.insert(Label::new(label_str), snapshot_id);
        }

        Ok(Self { snapshots, labels })
    }

    pub fn save(&self, path: &Path) -> Result<(), PricingError> {
        let mut persisted_labels: BTreeMap<String, SnapshotId> = BTreeMap::new();
        for (label, snapshot_id) in &self.labels {
            persisted_labels.insert(label.as_str().to_string(), *snapshot_id);
        }
        let mut entries: Vec<PersistedSnapshot> = self
            .snapshots
            .iter()
            .map(|(id, s)| PersistedSnapshot {
                snapshot_id: *id,
                source_url: s.source_url.clone(),
                note: s.note.clone(),
                prices: s.prices.clone(),
            })
            .collect();
        entries.sort_by_key(|e| e.snapshot_id);
        let persisted = PersistedRegistry {
            labels: persisted_labels,
            snapshots: entries,
        };
        let text = toml::to_string_pretty(&persisted)?;
        std::fs::write(path, text).map_err(|e| PricingError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    pub fn by_id(&self, snapshot_id: &SnapshotId) -> Option<&Snapshot> {
        self.snapshots.get(snapshot_id)
    }

    pub fn by_label(&self, label: &Label) -> Option<(&SnapshotId, &Snapshot)> {
        let id = self.labels.get(label)?;
        self.snapshots.get(id).map(|s| (id, s))
    }

    /// Resolve a snapshot by explicit id when provided, otherwise via
    /// `fallback_label`. Returns `UnknownSnapshotId` or `UnknownLabel`
    /// when the requested handle is absent.
    pub fn by_id_or_label(
        &self,
        snapshot_id: Option<SnapshotId>,
        fallback_label: &Label,
    ) -> Result<(SnapshotId, &Snapshot), PricingError> {
        match snapshot_id {
            Some(id) => self
                .by_id(&id)
                .map(|s| (id, s))
                .ok_or(PricingError::UnknownSnapshotId(id)),
            None => self
                .by_label(fallback_label)
                .map(|(id, s)| (*id, s))
                .ok_or_else(|| PricingError::UnknownLabel(fallback_label.clone())),
        }
    }

    /// Insert a snapshot at `snapshot_id`, and assign `labels` to it. Errors
    /// if the id is already present — existing snapshots are immutable.
    /// Each label listed is moved off whichever snapshot previously held it.
    pub fn insert(
        &mut self,
        snapshot_id: SnapshotId,
        snapshot: Snapshot,
        labels: &[Label],
    ) -> Result<(), PricingError> {
        if self.snapshots.contains_key(&snapshot_id) {
            return Err(PricingError::DuplicateSnapshotId(snapshot_id));
        }
        for label in labels {
            self.labels.insert(label.clone(), snapshot_id);
        }
        self.snapshots.insert(snapshot_id, snapshot);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn usd(s: &str) -> Usd {
        s.parse().expect("valid Usd")
    }

    fn sample_id() -> SnapshotId {
        SnapshotId::new(DateTime::from_timestamp(1_700_000_000, 0).expect("valid"))
    }

    fn sample_snapshot() -> Snapshot {
        let mut prices = BTreeMap::new();
        prices.insert(
            "fast-cheap".to_string(),
            ModelPrice {
                input_per_million: usd("1.00"),
                output_per_million: usd("2.00"),
            },
        );
        prices.insert(
            "slow-expensive".to_string(),
            ModelPrice {
                input_per_million: usd("10.00"),
                output_per_million: usd("30.00"),
            },
        );
        Snapshot {
            source_url: "https://models.dev/api.json".into(),
            note: None,
            prices,
        }
    }

    #[test]
    fn cost_combines_input_and_output_per_million() {
        // 1M input @ $1.00 + 100k output @ $2.00 = $1.00 + $0.20 = $1.20
        let cost = sample_snapshot()
            .cost_for("fast-cheap", 1_000_000, 100_000)
            .expect("known model");
        assert_eq!(cost, Usd::from_str("1.20").expect("valid"));
    }

    #[test]
    fn cost_scales_with_token_counts() {
        let s = sample_snapshot();
        let small = s.cost_for("slow-expensive", 1_000, 1_000).expect("known");
        let large = s.cost_for("slow-expensive", 10_000, 10_000).expect("known");
        assert_eq!(large, small * 10u64);
    }

    #[test]
    fn unknown_model_errors() {
        assert!(matches!(
            sample_snapshot().cost_for("does-not-exist", 100, 100),
            Err(PricingError::UnknownModel(_))
        ));
    }

    #[test]
    fn embedded_registry_parses_and_has_current_label() {
        let registry = PricesRegistry::embedded().expect("embedded parse");
        assert!(
            registry.by_label(&Label::new("current")).is_some(),
            "embedded prices.toml must have a `current` label"
        );
    }

    #[test]
    fn snapshot_id_roundtrips_through_string() {
        let id = sample_id();
        let parsed: SnapshotId = id.to_string().parse().expect("roundtrip");
        assert_eq!(parsed, id);
    }

    #[test]
    fn snapshot_id_rejects_non_rfc3339() {
        assert!("not a datetime".parse::<SnapshotId>().is_err());
    }

    #[test]
    fn registry_roundtrip_with_label() {
        let mut registry = PricesRegistry::new();
        let id = sample_id();
        let snapshot = sample_snapshot();
        registry
            .insert(id, snapshot.clone(), &[Label::new("current")])
            .expect("insert");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prices.toml");
        registry.save(&path).expect("save");
        let loaded = PricesRegistry::load(&path).expect("load");

        let (resolved_id, resolved) = loaded.by_label(&Label::new("current")).expect("resolves");
        assert_eq!(resolved_id, &id);
        assert_eq!(resolved, &snapshot);
    }

    #[test]
    fn insert_moves_label_to_newer_snapshot() {
        let mut registry = PricesRegistry::new();
        let older = SnapshotId::new(DateTime::from_timestamp(1_700_000_000, 0).expect("valid"));
        let newer = SnapshotId::new(DateTime::from_timestamp(1_700_001_000, 0).expect("valid"));
        registry
            .insert(older, sample_snapshot(), &[Label::new("current")])
            .expect("insert old");
        registry
            .insert(newer, sample_snapshot(), &[Label::new("current")])
            .expect("insert new");

        let (resolved_id, _) = registry.by_label(&Label::new("current")).expect("resolves");
        assert_eq!(resolved_id, &newer);
        assert!(registry.by_id(&older).is_some());
    }

    #[test]
    fn insert_rejects_duplicate_id() {
        let mut registry = PricesRegistry::new();
        let id = sample_id();
        registry.insert(id, sample_snapshot(), &[]).expect("insert");
        let err = registry
            .insert(id, sample_snapshot(), &[])
            .expect_err("dup");
        assert!(matches!(err, PricingError::DuplicateSnapshotId(_)));
    }

    #[test]
    fn load_rejects_unknown_label_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prices.toml");
        let content = r#"
[labels]
current = "2026-05-10T17:06:34.843738Z"
"#;
        std::fs::write(&path, content).expect("write");
        let err = PricesRegistry::load(&path).expect_err("must reject");
        assert!(matches!(err, PricingError::UnknownLabelSnapshot { .. }));
    }
}
