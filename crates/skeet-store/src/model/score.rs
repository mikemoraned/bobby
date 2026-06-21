use std::collections::HashMap;

use shared::{ImageId, ModelVersion, Score};

/// The full scores table, keyed by image id — the value cached by the adapter's
/// scored-view cache, gated on the scores table version.
pub type ScoresMap = HashMap<ImageId, (Score, ModelVersion)>;
