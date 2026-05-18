use std::collections::HashMap;
use std::str::FromStr;

use eval::{EvalResults, EvalSplit};
use shared::Band;
use skeet_store::ImageId;

use crate::train::TrainError;

/// Verify the baseline was computed against the same split currently on disk;
/// return the computed split hash.
pub fn verify_baseline_matches_split(
    split: &EvalSplit,
    baseline: &EvalResults,
) -> Result<String, TrainError> {
    let split_hash = split.content_hash();
    if split_hash != baseline.split_config_hash {
        return Err(TrainError::SplitHashDrift {
            split_hash,
            baseline_hash: baseline.split_config_hash.clone(),
        });
    }
    Ok(split_hash)
}

pub fn label_train_items(
    train: &[String],
    band_by_id: &HashMap<ImageId, Band>,
) -> Result<Vec<(String, Band)>, TrainError> {
    train
        .iter()
        .map(|s| {
            let id = ImageId::from_str(s).map_err(|_| TrainError::InvalidImageId(s.clone()))?;
            let band = band_by_id
                .get(&id)
                .copied()
                .ok_or_else(|| TrainError::AppraisalMissing(s.clone()))?;
            Ok((s.clone(), band))
        })
        .collect()
}
