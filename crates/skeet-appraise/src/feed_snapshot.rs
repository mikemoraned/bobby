use std::collections::HashMap;
use std::sync::Arc;

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use shared::{Band, ImageId, RefineModels};
use skeet_publish::effective_band::{image_effective_band, skeet_effective_band};
use skeet_publish::{Limit, Order};
use skeet_store::{Score, SkeetId, StoreError};

use crate::AppraiseStore;
use crate::available_feeds::{
    AvailableFeeds, DiscoverError, FeedOption, PublishedListCatalogReader, UnknownFeed,
};

pub struct FeedItem {
    pub skeet_id: SkeetId,
    pub image_id: ImageId,
    pub score: Score,
    pub effective_band: Band,
    pub manual_image_band: Option<Band>,
    pub manual_skeet_band: Option<Band>,
    pub image_url_exists: bool,
    pub skeet_id_exists: bool,
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
/// that renders one. The available feeds are discovered fresh per request from
/// the publisher's catalog (so feeds published after startup are picked up), and
/// the published list to read is chosen from them (see [`FeedSnapshotSource::load`]).
pub struct FeedSnapshotSource {
    feeds: AvailableFeeds,
    store: Arc<dyn AppraiseStore>,
    models: Arc<RefineModels>,
}

impl FromRequestHead for FeedSnapshotSource {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        let get = |missing: &'static str| move || cot::Error::internal(missing);
        let reader = head
            .extensions
            .get::<Arc<PublishedListCatalogReader>>()
            .cloned()
            .ok_or_else(get(
                "PublishedListCatalogReader not found in request extensions",
            ))?;
        // An empty catalog (the publisher hasn't advertised any feeds yet) is a
        // 404 rather than a 500 — there's simply nothing to show, not an error.
        let feeds = reader.discover().await.map_err(|e| match e {
            DiscoverError::NoFeeds(_) => {
                cot::error::NotFound::with_message("no feeds available").into()
            }
            other => cot::Error::internal(format!("discovering feeds: {other}")),
        })?;
        Ok(Self {
            feeds,
            store: head
                .extensions
                .get::<Arc<dyn AppraiseStore>>()
                .cloned()
                .ok_or_else(get("store not found in request extensions"))?,
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

        let image_ids: Vec<ImageId> = published.iter().map(|p| p.image_id.clone()).collect();
        let (scores, image_appraisals, skeet_appraisals) = tokio::try_join!(
            self.store.list_scores_for_ids(&image_ids),
            self.store.list_all_image_appraisals(),
            self.store.list_all_skeet_appraisals(),
        )?;
        let image_bands: HashMap<ImageId, Band> = image_appraisals
            .into_iter()
            .map(|(id, a)| (id, a.band))
            .collect();
        let skeet_bands: HashMap<SkeetId, Band> = skeet_appraisals
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
                    image_url_exists: item.image_url_exists,
                    skeet_id_exists: item.skeet_id_exists,
                })
            })
            .collect();

        Ok(FeedSnapshot { items })
    }
}
