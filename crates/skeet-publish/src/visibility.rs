use std::collections::{HashMap, HashSet};

use shared::{Band, ImageId, RefineModels};
use skeet_store::{ModelVersion, Score, SkeetId, StoredImageSummary};

use crate::effective_band::{image_effective_band, skeet_visible_in_feed};

/// The data the feed-visibility policy reads: the scored entries plus the manual
/// band overrides and model registry needed to interpret each score.
pub trait FeedData {
    fn entries(&self) -> &[(StoredImageSummary, Score, ModelVersion)];
    /// Manual band override for an image, if one has been set.
    fn image_band(&self, image_id: &ImageId) -> Option<Band>;
    /// Manual band override for a skeet, if one has been set.
    fn skeet_band(&self, skeet_id: &SkeetId) -> Option<Band>;
    fn models(&self) -> &RefineModels;

    fn visible_skeet_ids(&self) -> HashSet<SkeetId> {
        let mut per_skeet: HashMap<&SkeetId, Vec<Band>> = HashMap::new();
        for (summary, score, model_version) in self.entries() {
            let band = image_effective_band(
                *score,
                model_version,
                self.models(),
                self.image_band(&summary.image_id),
            );
            per_skeet.entry(&summary.skeet_id).or_default().push(band);
        }

        per_skeet
            .into_iter()
            .filter(|(skeet_id, image_bands)| {
                skeet_visible_in_feed(self.skeet_band(skeet_id), image_bands)
            })
            .map(|(skeet_id, _)| skeet_id.clone())
            .collect()
    }

    /// Scored entries from visible skeets only ([`Self::visible_skeet_ids`]),
    /// deduplicated by skeet_id (preserving input order).
    fn visible_entries(&self) -> Vec<(StoredImageSummary, Score, ModelVersion)> {
        let visible = self.visible_skeet_ids();

        let mut seen = HashSet::new();
        self.entries()
            .iter()
            .filter(|(summary, _, _)| {
                summary.skeet_id.collection() == "app.bsky.feed.post"
                    && visible.contains(&summary.skeet_id)
            })
            .filter(|(summary, _, _)| seen.insert(summary.skeet_id.clone()))
            .cloned()
            .collect()
    }
}
