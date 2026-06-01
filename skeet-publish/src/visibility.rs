use std::collections::{HashMap, HashSet};

use shared::{Band, ImageId, RefineModels};
use skeet_store::{ModelVersion, Score, SkeetId, StoredImageSummary};

use crate::effective_band::image_score_is_positive;

/// The data the feed-visibility policy reads: the scored entries plus the manual
/// band overrides and model registry needed to interpret each score.
///
/// Implemented by `CachedFeed` (the library/serving path) and by the publisher's
/// own windowed-query data, so the policy is decoupled from where the data comes
/// from and from `FeedCache`.
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
/// Score-based visibility uses the per-model threshold from `feed.models()`.
/// Manual band overrides bypass the model lookup and use `Band::is_visible_in_feed`.
fn visible_skeet_ids<F: FeedData>(feed: &F) -> HashSet<SkeetId> {
    // For each image: manual override wins; otherwise use per-model positive check.
    // Track per-skeet whether every image clears its bar.
    let mut skeet_visible: HashMap<&SkeetId, bool> = HashMap::new();
    for (summary, score, model_version) in feed.entries() {
        let manual_image = feed.image_band(&summary.image_id);
        let image_ok = manual_image.map_or_else(
            || image_score_is_positive(*score, model_version, feed.models()),
            |band| band.is_visible_in_feed(),
        );
        let entry = skeet_visible.entry(&summary.skeet_id).or_insert(true);
        *entry = *entry && image_ok;
    }

    skeet_visible
        .into_iter()
        .filter(|(skeet_id, all_images_ok)| {
            if !all_images_ok {
                return false;
            }
            let manual_skeet = feed.skeet_band(skeet_id);
            manual_skeet.is_none_or(|b| b.is_visible_in_feed())
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
