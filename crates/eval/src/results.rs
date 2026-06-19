use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::ModelVersion;
use uuid::{NoContext, Timestamp, Uuid};

use crate::confusion::ConfusionMatrix;
use crate::metric_types::{F1, PinnedPrecision, Precision, Recall, RocAuc};
use crate::pricing::SnapshotId;
use crate::split::SplitId;
use crate::usd::Usd;

/// A UUIDv7 identifier for an entry in `eval-results.toml`. The high 48 bits
/// encode the Unix-ms timestamp at which the run was generated, so entries
/// sort by creation time when ordered lexicographically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(Uuid);

#[derive(Debug, thiserror::Error)]
#[error("invalid run_id: {0}")]
pub struct InvalidRunId(#[from] uuid::Error);

impl RunId {
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidRunId> {
        Ok(Self(Uuid::parse_str(&s.into())?))
    }

    /// Build a UUIDv7 whose timestamp prefix matches `run_at` (ms precision).
    /// Negative timestamps (pre-1970) are clamped to 0; the random tail is
    /// drawn from the crate's RNG.
    pub fn from_run_at(run_at: DateTime<Utc>) -> Self {
        let secs = u64::try_from(run_at.timestamp()).unwrap_or(0);
        let nanos = run_at.timestamp_subsec_nanos();
        Self(Uuid::new_v7(Timestamp::from_unix(NoContext, secs, nanos)))
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl FromStr for RunId {
    type Err = InvalidRunId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Free-text describing why a run was made (e.g. `"phase-3 baseline"`,
/// `"phase-4 gpt-4o-mini #1"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Purpose(String);

impl Purpose {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Purpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One evaluation of a `(model_version, split)` pair. Persisted as an
/// `[[runs]]` entry inside `EvalResultsLog`.
///
/// `resources` covers the scoring pass that produced `evaluation`.
/// `training` covers the prompt-refinement loop that produced the model under
/// evaluation — `Some` when the run originated from training, `None`
/// otherwise (e.g. a stand-alone `refine-eval`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRecord {
    pub run_id: RunId,
    pub run_at: DateTime<Utc>,
    pub model_version: ModelVersion,
    pub split_id: SplitId,
    pub price_snapshot_id: SnapshotId,
    pub purpose: Purpose,
    pub evaluation: Evaluation,
    pub resources: Resources,
    #[serde(default)]
    pub training: Option<Resources>,
}

impl RunRecord {
    /// Sum of `resources.cost` and `training.cost` (zero when no training
    /// phase). Use this for budget comparisons; use `resources.cost` alone
    /// when comparing candidates on inference cost.
    pub fn total_cost(&self) -> Usd {
        self.resources.cost + self.training.as_ref().map_or_else(Usd::zero, |t| t.cost)
    }
}

/// Quality metrics produced by scoring a model against a labelled test set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evaluation {
    pub precision: Precision,
    pub recall: Recall,
    pub f1: F1,
    pub roc_auc: Option<RocAuc>,
    pub pinned_precision: Option<PinnedPrecision>,
    pub confusion: ConfusionMatrix,
}

/// What the run consumed: tokens billed and the resulting dollar cost.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Resources {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(rename = "cost_usd")]
    pub cost: Usd,
}

#[derive(Debug, thiserror::Error)]
pub enum EvalResultsLogError {
    #[error("failed to read/write {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse eval-results.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize eval-results.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("duplicate run_id {0} — each run must appear once")]
    DuplicateRunId(RunId),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedLog {
    #[serde(default, rename = "runs")]
    runs: Vec<RunRecord>,
}

/// Append-only log of `RunRecord`s.
#[derive(Debug, Clone, Default)]
pub struct EvalResultsLog {
    runs: Vec<RunRecord>,
}

impl EvalResultsLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(path: &Path) -> Result<Self, EvalResultsLogError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| EvalResultsLogError::Io(path.display().to_string(), e))?;
        let persisted: PersistedLog = toml::from_str(&text)?;
        Self::from_persisted(persisted)
    }

    pub fn load_or_empty(path: &Path) -> Result<Self, EvalResultsLogError> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::new())
        }
    }

    fn from_persisted(persisted: PersistedLog) -> Result<Self, EvalResultsLogError> {
        let mut seen = std::collections::HashSet::new();
        for run in &persisted.runs {
            if !seen.insert(run.run_id) {
                return Err(EvalResultsLogError::DuplicateRunId(run.run_id));
            }
        }
        Ok(Self {
            runs: persisted.runs,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), EvalResultsLogError> {
        let mut runs = self.runs.clone();
        for r in &mut runs {
            r.resources.cost = r.resources.cost.round_dp(4);
            if let Some(t) = &mut r.training {
                t.cost = t.cost.round_dp(4);
            }
        }
        runs.sort_by(|a, b| a.run_at.cmp(&b.run_at));
        let persisted = PersistedLog { runs };
        let text = toml::to_string_pretty(&persisted)?;
        std::fs::write(path, text)
            .map_err(|e| EvalResultsLogError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    pub fn append(&mut self, run: RunRecord) -> Result<(), EvalResultsLogError> {
        if self.runs.iter().any(|r| r.run_id == run.run_id) {
            return Err(EvalResultsLogError::DuplicateRunId(run.run_id));
        }
        self.runs.push(run);
        Ok(())
    }

    pub fn runs(&self) -> &[RunRecord] {
        &self.runs
    }

    /// All runs whose `model_version` equals `mv`, in insertion order.
    pub fn for_model(&self, mv: &ModelVersion) -> Vec<&RunRecord> {
        self.runs
            .iter()
            .filter(|r| &r.model_version == mv)
            .collect()
    }

    /// The run with the largest score returned by `score`. Runs for which
    /// `score` returns `None` are skipped. Ties resolved by insertion order
    /// (first wins). Returns `None` if no run produces a comparable score.
    pub fn best_by<F, S>(&self, score: F) -> Option<&RunRecord>
    where
        F: Fn(&RunRecord) -> Option<S>,
        S: PartialOrd,
    {
        let mut best: Option<(&RunRecord, S)> = None;
        for r in &self.runs {
            if let Some(s) = score(r) {
                let take = best.as_ref().is_none_or(|(_, best_s)| s > *best_s);
                if take {
                    best = Some((r, s));
                }
            }
        }
        best.map(|(r, _)| r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::metric_types::Threshold;

    fn usd(s: &str) -> Usd {
        s.parse().expect("valid Usd")
    }

    fn run_id(n: u32) -> RunId {
        RunId::new(format!("01900000-0000-7000-8000-{n:012x}")).expect("valid uuid v7")
    }

    fn snapshot_id() -> SnapshotId {
        SnapshotId::new(DateTime::from_timestamp(1_700_000_000, 0).expect("valid"))
    }

    fn sample_run(rid: RunId, mv: &str, f1: f64) -> RunRecord {
        RunRecord {
            run_id: rid,
            run_at: DateTime::from_timestamp(1_700_000_000, 0).expect("valid"),
            model_version: ModelVersion::from(mv),
            split_id: SplitId::new("00112233445566778899aabbccddeeff").expect("valid"),
            price_snapshot_id: snapshot_id(),
            purpose: Purpose::new("test"),
            evaluation: Evaluation {
                precision: Precision::new(0.85).expect("valid"),
                recall: Recall::new(0.72).expect("valid"),
                f1: F1::new(f1).expect("valid"),
                roc_auc: Some(RocAuc::new(0.91).expect("valid")),
                pinned_precision: Some(PinnedPrecision {
                    threshold: Threshold::new(0.6).expect("valid"),
                    recall: Recall::new(0.70).expect("valid"),
                }),
                confusion: ConfusionMatrix {
                    true_pos: 100,
                    false_pos: 18,
                    true_neg: 200,
                    false_neg: 39,
                },
            },
            resources: Resources {
                input_tokens: 50000,
                output_tokens: 5000,
                cost: usd("0.175"),
            },
            training: None,
        }
    }

    #[test]
    fn log_roundtrip_with_optionals() {
        let mut log = EvalResultsLog::new();
        let mut r1 = sample_run(run_id(1), "v2:abc", 0.8);
        r1.training = Some(Resources {
            input_tokens: 200_000,
            output_tokens: 20_000,
            cost: usd("0.50"),
        });
        log.append(r1).expect("append");
        let mut r2 = sample_run(run_id(2), "v1", 0.7);
        r2.evaluation.roc_auc = None;
        r2.evaluation.pinned_precision = None;
        log.append(r2).expect("append");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("eval-results.toml");
        log.save(&path).expect("save");
        let loaded = EvalResultsLog::load(&path).expect("load");
        assert_eq!(loaded.runs().len(), 2);
        let first = &loaded.runs()[0];
        let training = first.training.as_ref().expect("training preserved");
        assert_eq!(training.input_tokens, 200_000);
        assert_eq!(loaded.runs()[1].training, None);
    }

    #[test]
    fn append_rejects_duplicate_run_id() {
        let mut log = EvalResultsLog::new();
        log.append(sample_run(run_id(1), "v2:abc", 0.8))
            .expect("append");
        let err = log
            .append(sample_run(run_id(1), "v2:def", 0.5))
            .expect_err("duplicate must fail");
        assert!(matches!(err, EvalResultsLogError::DuplicateRunId(_)));
    }

    #[test]
    fn for_model_filters_by_model_version() {
        let mut log = EvalResultsLog::new();
        let r1 = run_id(1);
        let r3 = run_id(3);
        log.append(sample_run(r1, "v2:abc", 0.8)).expect("append");
        log.append(sample_run(run_id(2), "v2:def", 0.7))
            .expect("append");
        log.append(sample_run(r3, "v2:abc", 0.9)).expect("append");

        let abc = ModelVersion::from("v2:abc");
        let filtered = log.for_model(&abc);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|r| r.run_id == r1 || r.run_id == r3));
    }

    #[test]
    fn best_by_picks_max() {
        let mut log = EvalResultsLog::new();
        let r2 = run_id(2);
        log.append(sample_run(run_id(1), "v2:abc", 0.6))
            .expect("append");
        log.append(sample_run(r2, "v2:abc", 0.9)).expect("append");
        log.append(sample_run(run_id(3), "v2:abc", 0.8))
            .expect("append");

        let best = log.best_by(|r| Some(r.evaluation.f1)).expect("non-empty");
        assert_eq!(best.run_id, r2);
    }

    #[test]
    fn best_by_skips_none() {
        let mut log = EvalResultsLog::new();
        log.append(sample_run(run_id(1), "v2:abc", 0.6))
            .expect("append");

        let best = log.best_by(|_| Option::<F1>::None);
        assert!(best.is_none());
    }

    #[test]
    fn from_run_at_encodes_timestamp_in_high_48_bits() {
        let t = DateTime::from_timestamp(1_700_000_000, 0).expect("valid");
        let uuid = RunId::from_run_at(t).as_uuid();
        let bytes = *uuid.as_bytes();
        let unix_ms = u64::from_be_bytes([
            0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        ]);
        assert_eq!(unix_ms, 1_700_000_000 * 1000);
        assert_eq!(uuid.get_version_num(), 7);
    }

    #[test]
    fn from_run_at_unique_per_call() {
        let t = DateTime::from_timestamp(1_700_000_000, 0).expect("valid");
        assert_ne!(RunId::from_run_at(t), RunId::from_run_at(t));
    }

    #[test]
    fn from_str_rejects_non_uuid() {
        assert!("not-a-uuid".parse::<RunId>().is_err());
    }
}
