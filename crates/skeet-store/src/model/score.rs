use std::collections::HashMap;

use shared::{ImageId, ModelVersion, Score};

use crate::StoredImageSummary;

/// A model score paired with the `ModelVersion` that produced it.
///
/// The score is only meaningful alongside its version — the feed read path keeps
/// or discards a score by whether its version is currently known (see
/// `docs/versioning.md`), so the two always travel together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelScore {
    pub score: Score,
    pub model_version: ModelVersion,
}

/// The full scores table, keyed by image id — the value cached by the adapter's
/// scored-view cache, gated on the scores table version.
pub type ScoresMap = HashMap<ImageId, ModelScore>;

/// A stored image summary joined with its [`ModelScore`] — one row of a
/// scored read-model ([`crate::ScoredView`]).
#[derive(Clone)]
pub struct ScoredSummary {
    pub summary: StoredImageSummary,
    pub scored: ModelScore,
}
