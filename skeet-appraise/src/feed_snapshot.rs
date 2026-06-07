use std::collections::HashMap;

use shared::{Band, ImageId, RefineModels};
use skeet_publish::RedisFeedSource;
use skeet_publish::effective_band::image_effective_band;
use skeet_store::{Score, SkeetId, SkeetStore, StoreError};

pub struct FeedItem {
    pub skeet_id: SkeetId,
    pub image_id: ImageId,
    pub score: Score,
    pub effective_band: Band,
    pub manual_image_band: Option<Band>,
    pub manual_skeet_band: Option<Band>,
}

#[derive(Debug, thiserror::Error)]
pub enum FeedSnapshotError {
    #[error("failed to read published feed: {0}")]
    Feed(#[from] skeet_publish::FeedSourceError),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

pub struct FeedSnapshot {
    pub items: Vec<FeedItem>,
}

impl FeedSnapshot {
    /// Read the published list, then look up score + manual bands for **exactly**
    /// the published `image_id`s (a targeted lookup, not a capped bulk fetch) and
    /// join them. Items with no current score are dropped.
    pub async fn load(
        feed: &RedisFeedSource,
        store: &SkeetStore,
        models: &RefineModels,
    ) -> Result<Self, FeedSnapshotError> {
        let (published, _refreshed_at) = feed.published().await?;

        let image_id_strs: Vec<String> = published.iter().map(|p| p.image_id.to_string()).collect();
        let id_refs: Vec<&str> = image_id_strs.iter().map(String::as_str).collect();
        let scores = store.list_scores_for_ids(&id_refs).await?;
        let image_bands: HashMap<ImageId, Band> = store
            .list_all_image_appraisals()
            .await?
            .into_iter()
            .map(|(id, a)| (id, a.band))
            .collect();
        let skeet_bands: HashMap<SkeetId, Band> = store
            .list_all_skeet_appraisals()
            .await?
            .into_iter()
            .map(|(id, a)| (id, a.band))
            .collect();

        let items = published
            .into_iter()
            .filter_map(|item| {
                let (score, model_version) = scores.get(&item.image_id)?;
                let manual_image_band = image_bands.get(&item.image_id).copied();
                let manual_skeet_band = skeet_bands.get(&item.skeet_id).copied();
                let effective_band =
                    image_effective_band(*score, model_version, models, manual_image_band);
                Some(FeedItem {
                    skeet_id: item.skeet_id,
                    image_id: item.image_id,
                    score: *score,
                    effective_band,
                    manual_image_band,
                    manual_skeet_band,
                })
            })
            .collect();

        Ok(Self { items })
    }
}
