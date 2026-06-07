use std::collections::HashMap;
use std::sync::Arc;

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use shared::{Band, ImageId, RefineModels};
use skeet_publish::effective_band::{image_effective_band, skeet_effective_band};
use skeet_publish::{Limit, Order};
use skeet_store::{Score, SkeetId, SkeetStore, StoreError};

use crate::available_feeds::{AvailableFeeds, FeedOption, UnknownFeed};

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
    #[error("no reader configured for feed {0}-{1}")]
    UnknownFeed(Order, Limit),
}

pub struct FeedSnapshot {
    pub items: Vec<FeedItem>,
}

/// Everything needed to build a [`FeedSnapshot`], gathered from request extensions.
///
/// Handlers depend on this one extractor instead of the feeds/store/models trio,
/// so adding a new input to snapshot loading happens here, not in every handler
/// that renders one. The published list to read is chosen per request (see
/// [`FeedSnapshotSource::load`]) from the configured [`AvailableFeeds`].
pub struct FeedSnapshotSource {
    feeds: Arc<AvailableFeeds>,
    store: Arc<SkeetStore>,
    models: Arc<RefineModels>,
}

impl FromRequestHead for FeedSnapshotSource {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        let get = |missing: &'static str| move || cot::Error::internal(missing);
        Ok(Self {
            feeds: head
                .extensions
                .get::<Arc<AvailableFeeds>>()
                .cloned()
                .ok_or_else(get("AvailableFeeds not found in request extensions"))?,
            store: head
                .extensions
                .get::<Arc<SkeetStore>>()
                .cloned()
                .ok_or_else(get("SkeetStore not found in request extensions"))?,
            models: head
                .extensions
                .get::<Arc<RefineModels>>()
                .cloned()
                .ok_or_else(get("RefineModels not found in request extensions"))?,
        })
    }
}

impl FeedSnapshotSource {
    /// Resolve a requested `?feed=` value to a configured spec. Absent uses the
    /// default; an explicit unknown value is an error.
    pub fn resolve(&self, requested: Option<&str>) -> Result<(Order, Limit), UnknownFeed> {
        self.feeds.resolve(requested)
    }

    /// The dropdown options for the configured feeds, marking `selected`.
    pub fn options(&self, selected: (Order, Limit)) -> Vec<FeedOption> {
        self.feeds.options(selected)
    }

    /// Read the chosen published list, then look up score + manual bands for
    /// **exactly** the published `image_id`s (a targeted lookup, not a capped bulk
    /// fetch) and join them, resolving each item's model-aware effective band.
    /// Items with no current score are dropped.
    pub async fn load(&self, spec: (Order, Limit)) -> Result<FeedSnapshot, FeedSnapshotError> {
        let feed = self
            .feeds
            .reader(spec)
            .ok_or(FeedSnapshotError::UnknownFeed(spec.0, spec.1))?;
        let (published, _refreshed_at) = feed.published().await?;

        let image_id_strs: Vec<String> = published.iter().map(|p| p.image_id.to_string()).collect();
        let id_refs: Vec<&str> = image_id_strs.iter().map(String::as_str).collect();
        let scores = self.store.list_scores_for_ids(&id_refs).await?;
        let image_bands: HashMap<ImageId, Band> = self
            .store
            .list_all_image_appraisals()
            .await?
            .into_iter()
            .map(|(id, a)| (id, a.band))
            .collect();
        let skeet_bands: HashMap<SkeetId, Band> = self
            .store
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
                let image_band =
                    image_effective_band(*score, model_version, &self.models, manual_image_band);
                // The feed-effective band caps the image's band with the manual skeet
                // override (`min`), matching what the feed/quality sort publishes. The
                // slice is non-empty, so `skeet_effective_band` is always `Some`.
                let effective_band =
                    skeet_effective_band(manual_skeet_band, &[image_band]).unwrap_or(image_band);
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

        Ok(FeedSnapshot { items })
    }
}
