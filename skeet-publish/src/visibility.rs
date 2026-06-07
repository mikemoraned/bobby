use std::collections::{HashMap, HashSet};

use shared::{Band, ImageId, RefineModels};
use skeet_store::{ModelVersion, Score, SkeetId, StoredImageSummary};

use crate::effective_band::{image_effective_band, skeet_effective_band};

/// The data the feed-visibility policy reads: the scored entries plus the manual
/// band overrides and model registry needed to interpret each score.
///
/// Implemented by the publisher's windowed-query data, so the policy is decoupled
/// from where the data comes from.
pub trait FeedData {
    fn entries(&self) -> &[(StoredImageSummary, Score, ModelVersion)];
    /// Manual band override for an image, if one has been set.
    fn image_band(&self, image_id: &ImageId) -> Option<Band>;
    /// Manual band override for a skeet, if one has been set.
    fn skeet_band(&self, skeet_id: &SkeetId) -> Option<Band>;
    fn models(&self) -> &RefineModels;
}

/// Compute the set of skeet IDs whose effective band makes them visible in the feed.
///
/// Each image's band is the model-aware [`image_effective_band`] (manual override,
/// else the score normalised by the producing model's threshold); a skeet is
/// visible iff its [`skeet_effective_band`] — the min of its manual override and
/// all image bands — clears feed visibility. Visibility and the quality sort thus
/// read the same band.
fn visible_skeet_ids<F: FeedData>(feed: &F) -> HashSet<SkeetId> {
    let mut per_skeet: HashMap<&SkeetId, Vec<Band>> = HashMap::new();
    for (summary, score, model_version) in feed.entries() {
        let band = image_effective_band(
            *score,
            model_version,
            feed.models(),
            feed.image_band(&summary.image_id),
        );
        per_skeet.entry(&summary.skeet_id).or_default().push(band);
    }

    per_skeet
        .into_iter()
        .filter(|(skeet_id, image_bands)| {
            skeet_effective_band(feed.skeet_band(skeet_id), image_bands)
                .is_some_and(Band::is_visible_in_feed)
        })
        .map(|(skeet_id, _)| skeet_id.clone())
        .collect()
}

/// Return scored entries filtered to only those from visible skeets,
/// deduplicated by skeet_id (preserving the input order).
pub fn visible_entries<F: FeedData>(feed: &F) -> Vec<(StoredImageSummary, Score, ModelVersion)> {
    let visible = visible_skeet_ids(feed);

    let mut seen = HashSet::new();
    feed.entries()
        .iter()
        .filter(|(summary, _, _)| {
            summary.skeet_id.collection() == "app.bsky.feed.post"
                && visible.contains(&summary.skeet_id)
        })
        .filter(|(summary, _, _)| seen.insert(summary.skeet_id.clone()))
        .cloned()
        .collect()
}
