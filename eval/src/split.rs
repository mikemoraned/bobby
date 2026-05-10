use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::Band;
use smartcore::linalg::basic::matrix::DenseMatrix;
use smartcore::model_selection::train_test_split;

/// Frozen train/test split written to `config/eval-split.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvalSplit {
    pub seed: u64,
    pub captured_at: DateTime<Utc>,
    pub train: Vec<String>,
    pub test: Vec<String>,
}

impl EvalSplit {
    pub fn load(path: &std::path::Path) -> Result<Self, EvalSplitError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| EvalSplitError::Io(path.display().to_string(), e))?;
        toml::from_str(&content).map_err(EvalSplitError::Parse)
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), EvalSplitError> {
        let content = toml::to_string_pretty(self).map_err(EvalSplitError::Serialize)?;
        std::fs::write(path, content)
            .map_err(|e| EvalSplitError::Io(path.display().to_string(), e))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalSplitError {
    #[error("failed to read/write {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse eval-split.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize eval-split.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// Splits `items` into (train, test) by stratifying on `Band`, calling
/// `smartcore::model_selection::train_test_split` once per band so the seed
/// drives the per-band shuffle deterministically.
///
/// Bands with too few samples for the split (where smartcore's floor of
/// `n * test_size` would be 0) all fall into the train side.
pub fn stratified_split<T: Clone + ToString>(
    items: &[(T, Band)],
    train_ratio: f64,
    seed: u64,
) -> (Vec<T>, Vec<T>) {
    let test_size = 1.0 - train_ratio as f32;
    Band::ALL
        .iter()
        .map(|&band| {
            let group: Vec<T> = items
                .iter()
                .filter(|(_, b)| *b == band)
                .map(|(id, _)| id.clone())
                .collect();
            split_band(group, test_size, seed.wrapping_add(band as u64))
        })
        .fold(
            (Vec::new(), Vec::new()),
            |(mut train, mut test), (band_train, band_test)| {
                train.extend(band_train);
                test.extend(band_test);
                (train, test)
            },
        )
}

fn split_band<T: Clone>(group: Vec<T>, test_size: f32, seed: u64) -> (Vec<T>, Vec<T>) {
    let n = group.len();
    if (((n as f32) * test_size) as usize) < 1 {
        // smartcore requires at least 1 test sample; tiny bands all go to train.
        (group, Vec::new())
    } else {
        let dummy_x = DenseMatrix::new(n, 1, vec![0.0_f64; n], false)
            .expect("(n, 1) shape matches values len");
        let indices: Vec<u64> = (0..n as u64).collect();
        let (_, _, train_idx, test_idx) =
            train_test_split(&dummy_x, &indices, test_size, true, Some(seed));
        let train: Vec<T> = train_idx.into_iter().map(|i| group[i as usize].clone()).collect();
        let test: Vec<T> = test_idx.into_iter().map(|i| group[i as usize].clone()).collect();
        (train, test)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn make_items(bands: &[Band]) -> Vec<(String, Band)> {
        bands
            .iter()
            .enumerate()
            .map(|(i, b)| (format!("id-{i}"), *b))
            .collect()
    }

    fn enough_per_band() -> Vec<(String, Band)> {
        let bands = [
            Band::Low,
            Band::Low,
            Band::Low,
            Band::Low,
            Band::Low,
            Band::MediumLow,
            Band::MediumLow,
            Band::MediumLow,
            Band::MediumLow,
            Band::MediumLow,
            Band::MediumHigh,
            Band::MediumHigh,
            Band::MediumHigh,
            Band::MediumHigh,
            Band::MediumHigh,
            Band::HighQuality,
            Band::HighQuality,
            Band::HighQuality,
            Band::HighQuality,
            Band::HighQuality,
        ];
        make_items(&bands)
    }

    fn band_strategy() -> impl Strategy<Value = Band> {
        prop_oneof![
            Just(Band::Low),
            Just(Band::MediumLow),
            Just(Band::MediumHigh),
            Just(Band::HighQuality),
        ]
    }

    fn items_strategy(max_size: usize) -> impl Strategy<Value = Vec<(String, Band)>> {
        prop::collection::vec(band_strategy(), 0..=max_size).prop_map(|bands| make_items(&bands))
    }

    proptest! {
        /// Calling `stratified_split` twice with the same seed and inputs must produce
        /// identical (train, test) partitions for any items and any seed.
        #[test]
        fn split_is_deterministic(
            items in items_strategy(100),
            seed in any::<u64>(),
        ) {
            let (train1, test1) = stratified_split(&items, 0.8, seed);
            let (train2, test2) = stratified_split(&items, 0.8, seed);
            prop_assert_eq!(train1, train2);
            prop_assert_eq!(test1, test2);
        }
    }

    #[test]
    fn split_is_disjoint_and_covers_all() {
        let items = enough_per_band();
        let (train, test) = stratified_split(&items, 0.8, 1);
        let mut all: Vec<_> = train.iter().chain(test.iter()).cloned().collect();
        all.sort();
        let mut expected: Vec<_> = items.iter().map(|(id, _)| id.clone()).collect();
        expected.sort();
        assert_eq!(all, expected);
    }

    #[test]
    fn tiny_band_falls_through_to_train() {
        // Only 2 items in Low band; test_size=0.2 truncates to 0.
        let items = make_items(&[Band::Low, Band::Low]);
        let (train, test) = stratified_split(&items, 0.8, 0);
        assert_eq!(train.len(), 2);
        assert_eq!(test.len(), 0);
    }

    #[test]
    fn eval_split_roundtrip() {
        let split = EvalSplit {
            seed: 42,
            captured_at: DateTime::from_timestamp(0, 0).expect("valid timestamp"),
            train: vec!["a".into(), "b".into()],
            test: vec!["c".into()],
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("split.toml");
        split.save(&path).expect("save");
        let loaded = EvalSplit::load(&path).expect("load");
        assert_eq!(split, loaded);
    }
}
