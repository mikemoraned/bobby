use std::collections::{HashMap, HashSet};

use skeet_store::{ModelVersion, Score, SkeetId, StoredImageSummary};

use crate::effective_band::image_score_is_positive;
use crate::feed_cache::CachedFeed;

/// Compute the set of skeet IDs whose effective band makes them visible in the feed.
///
/// Score-based visibility uses the per-model threshold from `feed.models`.
/// Manual band overrides bypass the model lookup and use `Band::is_visible_in_feed`.
fn visible_skeet_ids(feed: &CachedFeed) -> HashSet<SkeetId> {
    // For each image: manual override wins; otherwise use per-model positive check.
    // Track per-skeet whether every image clears its bar.
    let mut skeet_visible: HashMap<&SkeetId, bool> = HashMap::new();
    for (summary, score, model_version) in &feed.entries {
        let manual_image = feed.image_appraisals.get(&summary.image_id).map(|a| a.band);
        let image_ok = manual_image.map_or_else(
            || image_score_is_positive(*score, model_version, &feed.models),
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
            let manual_skeet = feed.skeet_appraisals.get(skeet_id).map(|a| a.band);
            manual_skeet.is_none_or(|b| b.is_visible_in_feed())
        })
        .map(|(skeet_id, _)| skeet_id.clone())
        .collect()
}

/// Return scored entries filtered to only those from visible skeets,
/// sorted best-to-worst by score, deduplicated by skeet_id.
pub fn visible_entries(feed: &CachedFeed) -> Vec<(StoredImageSummary, Score, ModelVersion)> {
    let visible = visible_skeet_ids(feed);

    let mut seen = HashSet::new();
    feed.entries
        .iter()
        .filter(|(summary, _, _)| {
            summary.skeet_id.collection() == "app.bsky.feed.post"
                && visible.contains(&summary.skeet_id)
        })
        .filter(|(summary, _, _)| seen.insert(summary.skeet_id.clone()))
        .cloned()
        .collect()
}
