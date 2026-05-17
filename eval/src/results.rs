use serde::{Deserialize, Serialize};

use crate::metric_types::{F1, PinnedPrecision, Precision, Recall, RocAuc};
use crate::usd::Usd;

/// Full evaluation results for one model/prompt combination against the held-out test set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalResults {
    pub split_config_path: String,
    /// `EvalSplit::content_hash` of the split used — lets a later run detect drift.
    pub split_config_hash: String,
    pub model_version: String,
    pub model_name: String,
    pub precision: Precision,
    pub recall: Recall,
    pub f1: F1,
    /// `None` when AUC is undefined (only one class present in the test set).
    pub roc_auc: Option<RocAuc>,
    /// `None` when no threshold meets the target precision.
    pub pinned_precision: Option<PinnedPrecision>,
    pub tp: u64,
    pub fp: u64,
    pub tn: u64,
    pub fn_: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Usd,
}

impl EvalResults {
    pub fn load(path: &std::path::Path) -> Result<Self, EvalResultsError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| EvalResultsError::Io(path.display().to_string(), e))?;
        toml::from_str(&content).map_err(EvalResultsError::Parse)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), EvalResultsError> {
        let mut to_save = self.clone();
        to_save.cost_usd = to_save.cost_usd.round_dp(4);
        let content = toml::to_string_pretty(&to_save).map_err(EvalResultsError::Serialize)?;
        std::fs::write(path, content)
            .map_err(|e| EvalResultsError::Io(path.display().to_string(), e))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalResultsError {
    #[error("failed to read/write {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse eval-results: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize eval-results: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::metric_types::Threshold;

    fn usd(s: &str) -> Usd {
        s.parse().expect("valid Usd")
    }

    #[test]
    fn eval_results_roundtrip() {
        let results = EvalResults {
            split_config_path: "config/eval-split.toml".into(),
            split_config_hash: "abc123".into(),
            model_version: "v1".into(),
            model_name: "gpt-4o".into(),
            precision: Precision::new(0.85).expect("valid"),
            recall: Recall::new(0.72).expect("valid"),
            f1: F1::new(0.78).expect("valid"),
            roc_auc: Some(RocAuc::new(0.91).expect("valid")),
            pinned_precision: Some(PinnedPrecision {
                threshold: Threshold::new(0.6).expect("valid"),
                recall: Recall::new(0.70).expect("valid"),
            }),
            tp: 100,
            fp: 18,
            tn: 200,
            fn_: 39,
            input_tokens: 50000,
            output_tokens: 5000,
            cost_usd: usd("0.175"),
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("results.toml");
        results.save(&path).expect("save");
        let loaded = EvalResults::load(&path).expect("load");
        assert_eq!(results, loaded);
    }

    #[test]
    fn eval_results_roundtrip_with_none() {
        let results = EvalResults {
            split_config_path: "config/eval-split.toml".into(),
            split_config_hash: "abc123".into(),
            model_version: "v1".into(),
            model_name: "gpt-4o".into(),
            precision: Precision::new(0.0).expect("valid"),
            recall: Recall::new(0.0).expect("valid"),
            f1: F1::new(0.0).expect("valid"),
            roc_auc: None,
            pinned_precision: None,
            tp: 0,
            fp: 0,
            tn: 0,
            fn_: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: Usd::zero(),
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("results.toml");
        results.save(&path).expect("save");
        let loaded = EvalResults::load(&path).expect("load");
        assert_eq!(results, loaded);
    }
}
