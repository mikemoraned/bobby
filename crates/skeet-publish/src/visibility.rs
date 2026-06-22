use std::collections::{HashMap, HashSet};

use shared::{Band, ImageId, RefineModels, SkeetId};
use skeet_store::ScoredSummary;

use crate::effective_band::{image_effective_band, skeet_visible_in_feed};

/// The data the feed-visibility policy reads: the scored entries plus the manual
/// band overrides and model registry needed to interpret each score.
pub trait FeedData {
    fn entries(&self) -> &[ScoredSummary];
    /// Manual band override for an image, if one has been set.
    fn image_band(&self, image_id: &ImageId) -> Option<Band>;
    /// Manual band override for a skeet, if one has been set.
    fn skeet_band(&self, skeet_id: &SkeetId) -> Option<Band>;
    fn models(&self) -> &RefineModels;

    fn visible_skeet_ids(&self) -> HashSet<SkeetId> {
        let mut per_skeet: HashMap<&SkeetId, Vec<Band>> = HashMap::new();
        for entry in self.entries() {
            let band = image_effective_band(
                entry.scored.score,
                &entry.scored.model_version,
                self.models(),
                self.image_band(&entry.summary.image_id),
            );
            per_skeet
                .entry(&entry.summary.skeet_id)
                .or_default()
                .push(band);
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
    fn visible_entries(&self) -> Vec<ScoredSummary> {
        let visible = self.visible_skeet_ids();

        let mut seen = HashSet::new();
        self.entries()
            .iter()
            .filter(|entry| {
                entry.summary.skeet_id.collection() == "app.bsky.feed.post"
                    && visible.contains(&entry.summary.skeet_id)
            })
            .filter(|entry| seen.insert(entry.summary.skeet_id.clone()))
            .cloned()
            .collect()
    }
}
