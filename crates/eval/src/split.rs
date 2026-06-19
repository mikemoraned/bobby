use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use md5::Digest;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use shared::refine_model::Label;
use shared::{Band, ImageId};
use smartcore::linalg::basic::matrix::DenseMatrix;
use smartcore::model_selection::train_test_split;

/// Content-derived identifier for an `EvalSplit`. Wraps an `md5::Digest` so
/// the only ways to obtain a value are computing one from real content (via
/// `EvalSplit::id`) or parsing a 32-character hex string. Serialised as the
/// hex form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SplitId(Digest);

#[derive(Debug, thiserror::Error)]
#[error("invalid split_id (expected 32-character hex string): {0}")]
pub struct InvalidSplitId(String);

impl SplitId {
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidSplitId> {
        let s = s.into();
        let bytes: [u8; 16] = hex::decode(&s)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or(InvalidSplitId(s))?;
        Ok(Self(Digest(bytes)))
    }

    pub fn from_digest(digest: Digest) -> Self {
        Self(digest)
    }

    pub fn as_digest(&self) -> Digest {
        self.0
    }
}

impl FromStr for SplitId {
    type Err = InvalidSplitId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl std::fmt::Display for SplitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x}", self.0)
    }
}

impl Serialize for SplitId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SplitId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A frozen train/test split. Persisted as one `[[splits]]` entry inside
/// `EvalSplits`; not directly written or read on its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvalSplit {
    pub seed: u64,
    pub captured_at: DateTime<Utc>,
    pub train: Vec<ImageId>,
    pub test: Vec<ImageId>,
}

impl EvalSplit {
    /// Content-derived `SplitId`. Two splits are equal iff their `seed`,
    /// `captured_at`, and `train`/`test` id sequences are equal. Hashes a
    /// canonical byte stream — fields are length-prefixed so that moving
    /// items between `train` and `test` always changes the id.
    pub fn id(&self) -> SplitId {
        let mut ctx = md5::Context::new();
        ctx.consume(self.seed.to_le_bytes());
        ctx.consume(self.captured_at.timestamp_micros().to_le_bytes());
        consume_id_list(&mut ctx, &self.train);
        consume_id_list(&mut ctx, &self.test);
        SplitId::from_digest(ctx.compute())
    }
}

fn consume_id_list(ctx: &mut md5::Context, ids: &[ImageId]) {
    ctx.consume((ids.len() as u64).to_le_bytes());
    for id in ids {
        let s = id.to_string();
        ctx.consume((s.len() as u64).to_le_bytes());
        ctx.consume(s.as_bytes());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalSplitsError {
    #[error("failed to read/write {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse eval-splits.toml: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("failed to serialize eval-splits.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("split_id mismatch: stored {stored}, recomputed {recomputed}")]
    SplitIdMismatch {
        stored: SplitId,
        recomputed: SplitId,
    },
    #[error("label {label} references unknown split_id {split_id}")]
    UnknownLabelSplit { label: String, split_id: SplitId },
    #[error("duplicate split_id {0} — each split must appear once")]
    DuplicateSplitId(SplitId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSplit {
    split_id: SplitId,
    seed: u64,
    captured_at: DateTime<Utc>,
    train: Vec<ImageId>,
    test: Vec<ImageId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedSplits {
    #[serde(default)]
    labels: HashMap<String, SplitId>,
    #[serde(default, rename = "splits")]
    splits: Vec<PersistedSplit>,
}

/// Registry of `EvalSplit` entries keyed by `SplitId`, with named labels
/// (e.g. `default`) pointing at canonical splits.
#[derive(Debug, Clone, Default)]
pub struct EvalSplits {
    splits: HashMap<SplitId, EvalSplit>,
    labels: HashMap<Label, SplitId>,
}

impl EvalSplits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(path: &Path) -> Result<Self, EvalSplitsError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| EvalSplitsError::Io(path.display().to_string(), e))?;
        let persisted: PersistedSplits = toml::from_str(&text)?;
        Self::from_persisted(persisted)
    }

    pub fn load_or_empty(path: &Path) -> Result<Self, EvalSplitsError> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::new())
        }
    }

    fn from_persisted(persisted: PersistedSplits) -> Result<Self, EvalSplitsError> {
        let mut splits: HashMap<SplitId, EvalSplit> = HashMap::new();
        for entry in persisted.splits {
            let stored_id = entry.split_id;
            let split = EvalSplit {
                seed: entry.seed,
                captured_at: entry.captured_at,
                train: entry.train,
                test: entry.test,
            };
            let recomputed = split.id();
            if recomputed != stored_id {
                return Err(EvalSplitsError::SplitIdMismatch {
                    stored: stored_id,
                    recomputed,
                });
            }
            if splits.contains_key(&stored_id) {
                return Err(EvalSplitsError::DuplicateSplitId(stored_id));
            }
            splits.insert(stored_id, split);
        }

        let mut labels: HashMap<Label, SplitId> = HashMap::new();
        for (label_str, split_id) in persisted.labels {
            if !splits.contains_key(&split_id) {
                return Err(EvalSplitsError::UnknownLabelSplit {
                    label: label_str,
                    split_id,
                });
            }
            labels.insert(Label::new(label_str), split_id);
        }

        Ok(Self { splits, labels })
    }

    pub fn save(&self, path: &Path) -> Result<(), EvalSplitsError> {
        let mut persisted_labels: HashMap<String, SplitId> = HashMap::new();
        for (label, split_id) in &self.labels {
            persisted_labels.insert(label.as_str().to_string(), *split_id);
        }

        let mut entries: Vec<PersistedSplit> = self
            .splits
            .iter()
            .map(|(split_id, split)| PersistedSplit {
                split_id: *split_id,
                seed: split.seed,
                captured_at: split.captured_at,
                train: split.train.clone(),
                test: split.test.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.captured_at.cmp(&b.captured_at));

        let persisted = PersistedSplits {
            labels: persisted_labels,
            splits: entries,
        };
        let text = toml::to_string_pretty(&persisted)?;
        std::fs::write(path, text)
            .map_err(|e| EvalSplitsError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    pub fn by_id(&self, split_id: &SplitId) -> Option<&EvalSplit> {
        self.splits.get(split_id)
    }

    pub fn by_label(&self, label: &Label) -> Option<(&SplitId, &EvalSplit)> {
        let split_id = self.labels.get(label)?;
        self.splits.get(split_id).map(|s| (split_id, s))
    }

    /// Insert a split, deriving its id from content, and assign the listed
    /// labels to it. Any label listed is moved to the new entry's id, removing
    /// it from whichever entry previously held it. Returns the assigned id.
    pub fn insert(&mut self, split: EvalSplit, labels: &[Label]) -> SplitId {
        let split_id = split.id();
        for label in labels {
            self.labels.insert(label.clone(), split_id);
        }
        self.splits.insert(split_id, split);
        split_id
    }
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

/// Draw a per-band-stratified subsample of approximately `sample_size` items.
///
/// Each band is allocated a proportional quota (rounded, minimum 1 if the
/// band has any items), then `smartcore::model_selection::train_test_split`
/// shuffles within the band and the quota is taken from the test partition.
/// The seed offset per band matches `stratified_split` so the two functions
/// behave consistently for the same `seed`.
pub fn stratified_sample<T: Clone>(items: &[(T, Band)], sample_size: usize, seed: u64) -> Vec<T> {
    if items.is_empty() || sample_size == 0 {
        return Vec::new();
    }
    let total = items.len();
    Band::ALL
        .iter()
        .flat_map(|&band| {
            let group: Vec<T> = items
                .iter()
                .filter(|(_, b)| *b == band)
                .map(|(id, _)| id.clone())
                .collect();
            sample_band(group, sample_size, total, seed.wrapping_add(band as u64))
        })
        .collect()
}

// `DenseMatrix::new` is given a matching `(n, 1)` shape and an `n`-length value vec, so it
// cannot fail.
#[allow(clippy::expect_used)]
fn sample_band<T: Clone>(group: Vec<T>, total_target: usize, total: usize, seed: u64) -> Vec<T> {
    let n = group.len();
    if n == 0 {
        return Vec::new();
    }
    let quota = (((total_target as f64) * (n as f64) / (total as f64)).round() as usize)
        .max(1)
        .min(n);
    if quota == n {
        return group;
    }
    let test_size = quota as f32 / n as f32;
    let dummy_x =
        DenseMatrix::new(n, 1, vec![0.0_f64; n], false).expect("(n, 1) shape matches values len");
    let indices: Vec<u64> = (0..n as u64).collect();
    let (_, _, _, test_idx) = train_test_split(&dummy_x, &indices, test_size, true, Some(seed));
    test_idx
        .into_iter()
        .map(|i| group[i as usize].clone())
        .collect()
}

// `DenseMatrix::new` is given a matching `(n, 1)` shape and an `n`-length value vec, so it
// cannot fail.
#[allow(clippy::expect_used)]
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
        let train: Vec<T> = train_idx
            .into_iter()
            .map(|i| group[i as usize].clone())
            .collect();
        let test: Vec<T> = test_idx
            .into_iter()
            .map(|i| group[i as usize].clone())
            .collect();
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
        let items = make_items(&[Band::Low, Band::Low]);
        let (train, test) = stratified_split(&items, 0.8, 0);
        assert_eq!(train.len(), 2);
        assert_eq!(test.len(), 0);
    }

    fn id(n: u32) -> ImageId {
        format!("00000000-0000-0000-0000-{n:012x}")
            .parse()
            .expect("valid v1 image id")
    }

    fn sample_split() -> EvalSplit {
        EvalSplit {
            seed: 42,
            captured_at: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp"),
            train: vec![id(1), id(2)],
            test: vec![id(3)],
        }
    }

    #[test]
    fn id_is_stable() {
        let split = sample_split();
        assert_eq!(split.id(), split.clone().id());
    }

    #[test]
    fn id_changes_when_any_logical_field_changes() {
        let base = sample_split().id();

        let mut other_seed = sample_split();
        other_seed.seed = 43;
        assert_ne!(base, other_seed.id());

        let mut other_time = sample_split();
        other_time.captured_at = DateTime::from_timestamp(1_700_000_001, 0).expect("valid");
        assert_ne!(base, other_time.id());

        let mut reordered = sample_split();
        reordered.train.reverse();
        assert_ne!(base, reordered.id());

        let mut moved = sample_split();
        moved.train.push(id(99));
        moved.test.clear();
        assert_ne!(base, moved.id());
    }

    proptest! {
        /// Calling `stratified_sample` twice with the same seed and inputs must produce
        /// an identical sample for any items, sample size, and seed.
        #[test]
        fn sample_is_deterministic(
            items in items_strategy(100),
            sample_size in 0usize..=50,
            seed in any::<u64>(),
        ) {
            let s1 = stratified_sample(&items, sample_size, seed);
            let s2 = stratified_sample(&items, sample_size, seed);
            prop_assert_eq!(s1, s2);
        }

        /// Every item returned by `stratified_sample` must be one of the input items.
        #[test]
        fn sample_is_a_subset(
            items in items_strategy(100),
            sample_size in 0usize..=50,
            seed in any::<u64>(),
        ) {
            let inputs: std::collections::HashSet<String> =
                items.iter().map(|(id, _)| id.clone()).collect();
            let sample = stratified_sample(&items, sample_size, seed);
            for id in &sample {
                prop_assert!(inputs.contains(id));
            }
        }

        /// `stratified_sample` returns distinct items (no duplicates).
        #[test]
        fn sample_has_no_duplicates(
            items in items_strategy(100),
            sample_size in 0usize..=50,
            seed in any::<u64>(),
        ) {
            let sample = stratified_sample(&items, sample_size, seed);
            let unique: std::collections::HashSet<_> = sample.iter().cloned().collect();
            prop_assert_eq!(sample.len(), unique.len());
        }
    }

    #[test]
    fn sample_size_zero_returns_empty() {
        let items = enough_per_band();
        assert!(stratified_sample(&items, 0, 1).is_empty());
    }

    #[test]
    fn sample_size_at_least_total_returns_everything() {
        let items = enough_per_band();
        let sample = stratified_sample(&items, items.len() * 2, 1);
        assert_eq!(sample.len(), items.len());
    }

    /// When `sample_size` hugely exceeds the pool — as when a cheap model's
    /// budget-derived per-iteration size (e.g. 4486) dwarfs the train pool
    /// (e.g. 588) — the sample must be the whole pool, each item exactly once:
    /// no replacement, no duplicates, no growth past the pool. Uses uneven band
    /// sizes (so per-band quota rounding is exercised) and an extreme multiplier
    /// mirroring the production ratio.
    #[test]
    fn oversized_sample_returns_each_input_exactly_once() {
        let mut bands: Vec<Band> = Vec::new();
        for (band, n) in [
            (Band::Low, 40),
            (Band::MediumLow, 7),
            (Band::MediumHigh, 25),
            (Band::HighQuality, 3),
        ] {
            bands.extend(std::iter::repeat_n(band, n));
        }
        let items = make_items(&bands);

        let sample = stratified_sample(&items, items.len() * 8, 7);

        let mut got: Vec<String> = sample;
        got.sort();
        let mut expected: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
        expected.sort();
        assert_eq!(
            got, expected,
            "oversized request must return the full pool, each item once"
        );
    }

    #[test]
    fn sample_includes_at_least_one_from_each_non_empty_band() {
        let items = enough_per_band();
        let bands_in_sample: std::collections::HashSet<_> = stratified_sample(&items, 4, 1)
            .into_iter()
            .map(|id| {
                items
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map(|(_, b)| *b)
                    .expect("sampled id was an input")
            })
            .collect();
        assert_eq!(bands_in_sample.len(), Band::ALL.len());
    }

    fn write_splits(s: &EvalSplits) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("eval-splits.toml");
        s.save(&path).expect("save");
        dir
    }

    #[test]
    fn registry_roundtrip_with_label() {
        let split = sample_split();
        let expected_id = split.id();
        let mut registry = EvalSplits::new();
        let assigned = registry.insert(split.clone(), &[Label::new("default")]);
        assert_eq!(assigned, expected_id);

        let dir = write_splits(&registry);
        let path = dir.path().join("eval-splits.toml");
        let loaded = EvalSplits::load(&path).expect("load");

        let (resolved_id, resolved_split) = loaded
            .by_label(&Label::new("default"))
            .expect("default resolves");
        assert_eq!(resolved_id, &expected_id);
        assert_eq!(resolved_split, &split);
    }

    #[test]
    fn load_rejects_split_id_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("eval-splits.toml");
        let split = sample_split();
        let content = format!(
            r#"
[labels]

[[splits]]
split_id = "deadbeefdeadbeefdeadbeefdeadbeef"
seed = {seed}
captured_at = "{captured_at}"
train = ["{train1}", "{train2}"]
test = ["{test1}"]
"#,
            seed = split.seed,
            captured_at = split.captured_at.to_rfc3339(),
            train1 = split.train[0],
            train2 = split.train[1],
            test1 = split.test[0],
        );
        std::fs::write(&path, content).expect("write");
        let err = EvalSplits::load(&path).expect_err("must reject");
        assert!(matches!(err, EvalSplitsError::SplitIdMismatch { .. }));
    }

    #[test]
    fn load_rejects_unknown_label_split() {
        let mut registry = EvalSplits::new();
        let assigned = registry.insert(sample_split(), &[Label::new("default")]);
        let dir = write_splits(&registry);
        let path = dir.path().join("eval-splits.toml");
        let original = std::fs::read_to_string(&path).expect("read");
        let other = "deadbeefdeadbeefdeadbeefdeadbeef";
        assert_ne!(assigned.to_string(), other);
        let mutated = original.replacen(&assigned.to_string(), other, 1);
        std::fs::write(&path, mutated).expect("write");

        let err = EvalSplits::load(&path).expect_err("must reject");
        assert!(matches!(err, EvalSplitsError::UnknownLabelSplit { .. }));
    }

    #[test]
    fn id_changes_when_image_moves_between_train_and_test() {
        let a = EvalSplit {
            seed: 1,
            captured_at: DateTime::from_timestamp(0, 0).expect("valid"),
            train: vec![id(1)],
            test: vec![],
        };
        let b = EvalSplit {
            seed: 1,
            captured_at: DateTime::from_timestamp(0, 0).expect("valid"),
            train: vec![],
            test: vec![id(1)],
        };
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn split_id_display_is_32_hex_chars() {
        let s = sample_split().id().to_string();
        assert_eq!(s.len(), 32);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn split_id_parse_rejects_wrong_length() {
        assert!("c393ad1ec67c1744".parse::<SplitId>().is_err());
        assert!(
            "not hex at all not hex at all !!"
                .parse::<SplitId>()
                .is_err()
        );
    }

    #[test]
    fn insert_moves_label_to_new_split() {
        let mut registry = EvalSplits::new();
        let old_split = sample_split();
        let mut new_split = sample_split();
        new_split.captured_at = DateTime::from_timestamp(1_700_000_500, 0).expect("valid");
        let old_id = registry.insert(old_split, &[Label::new("default")]);
        let new_id = registry.insert(new_split, &[Label::new("default")]);
        assert_ne!(old_id, new_id);

        let (resolved_id, _) = registry
            .by_label(&Label::new("default"))
            .expect("default resolves");
        assert_eq!(resolved_id, &new_id);
        // Old entry is still present.
        assert!(registry.by_id(&old_id).is_some());
    }
}
