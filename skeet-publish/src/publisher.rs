use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use deadpool_redis::redis;
use shared::{ImageId, RefineModels};
use skeet_store::{
    Appraisal, ModelVersion, Score, SkeetId, SkeetStore, StoreError, StoredImageSummary, Version,
};
use tokio::sync::RwLock;

use crate::image_url_resolver::ImageUrlResolver;
use crate::limit::Limit;
use crate::order::Order;
use crate::published_list::{PublishedList, PublishedListError};
use crate::published::Published;
use crate::table_watch::relevant;
use crate::visibility::{FeedData, visible_entries};

/// The publisher's snapshot of scored skeets in a recency window, plus the
/// manual appraisals and models the visibility policy needs.
///
/// Assembled from an uncapped, recency-windowed store query, and implements
/// [`FeedData`] so the shared visibility policy runs over it.
pub struct WindowedFeed {
    pub entries: Vec<(StoredImageSummary, Score, ModelVersion)>,
    pub skeet_appraisals: HashMap<SkeetId, Appraisal>,
    pub image_appraisals: HashMap<ImageId, Appraisal>,
    pub models: Arc<RefineModels>,
}

impl FeedData for WindowedFeed {
    fn entries(&self) -> &[(StoredImageSummary, Score, ModelVersion)] {
        &self.entries
    }

    fn image_band(&self, image_id: &ImageId) -> Option<shared::Band> {
        self.image_appraisals.get(image_id).map(|a| a.band)
    }

    fn skeet_band(&self, skeet_id: &SkeetId) -> Option<shared::Band> {
        self.skeet_appraisals.get(skeet_id).map(|a| a.band)
    }

    fn models(&self) -> &RefineModels {
        self.models.as_ref()
    }
}

/// Compute the published pairs for one `(order, limit)` spec from a feed.
///
/// Reuses the visibility policy ([`visible_entries`]) to choose *which* skeets
/// are allowed, then applies the spec's ordering and window: for
/// [`Order::Recency`], keep only entries published within `limit.window()` of
/// `now` and sort newest-first by `original_at`. Each surviving entry's
/// representative image is resolved to a CDN url; entries whose image can't be
/// resolved (non-`V3` ids) are dropped.
pub fn published_for_spec<F: FeedData>(
    feed: &F,
    order: Order,
    limit: Limit,
    resolver: &dyn ImageUrlResolver,
    now: DateTime<Utc>,
) -> Vec<Published> {
    let cutoff_us = (now - limit.window()).timestamp_micros();

    let mut windowed: Vec<(StoredImageSummary, Score, ModelVersion)> = visible_entries(feed)
        .into_iter()
        .filter(|(summary, _, _)| summary.original_at.timestamp_micros() >= cutoff_us)
        .collect();

    match order {
        Order::Recency => windowed.sort_by(|a, b| {
            b.0.original_at
                .timestamp_micros()
                .cmp(&a.0.original_at.timestamp_micros())
        }),
    }

    windowed
        .into_iter()
        .filter_map(|(summary, _, _)| {
            resolver
                .resolve(&summary.skeet_id, &summary.image_id)
                .map(|image_url| Published {
                    image_url,
                    image_id: summary.image_id,
                    skeet_id: summary.skeet_id,
                })
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error(transparent)]
    List(#[from] PublishedListError),
}

/// Whether a `publish_if_changed` cycle did work.
#[derive(Debug)]
pub enum PublishOutcome {
    /// No relevant table version moved since the last publish — nothing written.
    Unchanged,
    /// The lists were recomputed and republished
    Published(Vec<(Order, Limit)>),
}

/// Publishes one redis list per `(Order, Limit)` spec from the live store.
///
/// On each cycle it queries the scored skeets published within the widest spec
/// window, runs the visibility policy, and writes each spec's ordered,
/// windowed pairs to its `{order}-{limit}` list.
pub struct FeedPublisher {
    store: Arc<SkeetStore>,
    models: Arc<RefineModels>,
    resolver: Arc<dyn ImageUrlResolver>,
    specs: Vec<(Order, Limit)>,
    /// The store version snapshot at the last publish, for change-gating.
    last_snapshot: RwLock<Option<HashSet<Version>>>,
}

impl FeedPublisher {
    pub fn new(
        store: Arc<SkeetStore>,
        models: Arc<RefineModels>,
        resolver: Arc<dyn ImageUrlResolver>,
        specs: Vec<(Order, Limit)>,
    ) -> Self {
        Self {
            store,
            models,
            resolver,
            specs,
            last_snapshot: RwLock::new(None),
        }
    }

    /// Fetch the scored skeets published within the widest spec window, plus the
    /// current manual appraisals.
    async fn fetch(&self, now: DateTime<Utc>) -> Result<WindowedFeed, StoreError> {
        let widest = self
            .specs
            .iter()
            .map(|(_, limit)| limit.window())
            .max()
            .unwrap_or_else(chrono::Duration::zero);

        let entries = self
            .store
            .list_scored_summaries_published_since(now - widest)
            .await?;
        let skeet_appraisals = self
            .store
            .list_all_skeet_appraisals()
            .await?
            .into_iter()
            .collect();
        let image_appraisals = self
            .store
            .list_all_image_appraisals()
            .await?
            .into_iter()
            .collect();

        Ok(WindowedFeed {
            entries,
            skeet_appraisals,
            image_appraisals,
            models: Arc::clone(&self.models),
        })
    }

    /// Compute and atomically publish every spec's list to redis.
    pub async fn publish<C>(&self, conn: &mut C, now: DateTime<Utc>) -> Result<(), PublishError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let feed = self.fetch(now).await?;
        for (order, limit) in &self.specs {
            let pairs = published_for_spec(&feed, *order, *limit, self.resolver.as_ref(), now);
            PublishedList::new(*order, *limit)
                .replace(conn, &pairs, now)
                .await?;
        }
        Ok(())
    }

    /// Publish only if a relevant table version (scores or appraisals) has moved
    /// since the last publish — so an idle worker skips the full store fetch and
    /// redis writes when nothing the feed depends on has changed.
    pub async fn publish_if_changed<C>(
        &self,
        conn: &mut C,
        now: DateTime<Utc>,
    ) -> Result<PublishOutcome, PublishError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let snapshot = self.store.version_snapshot().await?;
        let changed = {
            let guard = self.last_snapshot.read().await;
            guard
                .as_ref()
                .is_none_or(|prev| relevant(prev) != relevant(&snapshot))
        };
        if !changed {
            return Ok(PublishOutcome::Unchanged);
        }

        self.publish(conn, now).await?;
        *self.last_snapshot.write().await = Some(snapshot);
        Ok(PublishOutcome::Published(self.specs.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::{Appraiser, Band, BlueskyCid};
    use skeet_store::{DiscoveredAt, OriginalAt, Zone};
    use test_support::test_models;

    use crate::image_url_resolver::CdnImageUrlResolver;

    const CID: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

    /// A scored entry for skeet `rkey`, published `published`, with a positive
    /// score (≥ 0.5 for the `test` model) and a `V3` image id so the CDN
    /// resolver succeeds.
    fn entry(
        rkey: &str,
        published: DateTime<Utc>,
        score: f32,
    ) -> (StoredImageSummary, Score, ModelVersion) {
        let summary = StoredImageSummary {
            image_id: ImageId::V3(BlueskyCid::new(CID).expect("valid cid")),
            skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
                .parse()
                .expect("valid skeet id"),
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(published),
            zone: Zone::TopRight,
            config_version: ModelVersion::from("test"),
            detected_text: String::new(),
        };
        (
            summary,
            Score::new(score).expect("valid score"),
            ModelVersion::from("test"),
        )
    }

    fn feed(entries: Vec<(StoredImageSummary, Score, ModelVersion)>) -> WindowedFeed {
        WindowedFeed {
            entries,
            skeet_appraisals: HashMap::new(),
            image_appraisals: HashMap::new(),
            models: test_models(),
        }
    }

    fn skeet_rkeys(pairs: &[Published]) -> Vec<String> {
        pairs
            .iter()
            .map(|p| p.skeet_id.rkey().as_str().to_string())
            .collect()
    }

    #[test]
    fn orders_by_recency_newest_first() {
        let now = Utc::now();
        let feed = feed(vec![
            entry("old", now - chrono::Duration::hours(10), 0.9),
            entry("newest", now - chrono::Duration::hours(1), 0.6),
            entry("middle", now - chrono::Duration::hours(5), 0.7),
        ]);
        let pairs = published_for_spec(
            &feed,
            Order::Recency,
            Limit::hours(48),
            &CdnImageUrlResolver,
            now,
        );
        // Recency order, not score order (score would be old/middle/newest).
        assert_eq!(skeet_rkeys(&pairs), ["newest", "middle", "old"]);
    }

    #[test]
    fn drops_entries_outside_the_window() {
        let now = Utc::now();
        let feed = feed(vec![
            entry("inside", now - chrono::Duration::hours(10), 0.9),
            entry("outside", now - chrono::Duration::hours(60), 0.9),
        ]);
        let pairs = published_for_spec(
            &feed,
            Order::Recency,
            Limit::hours(48),
            &CdnImageUrlResolver,
            now,
        );
        assert_eq!(skeet_rkeys(&pairs), ["inside"]);
    }

    #[test]
    fn drops_invisible_skeets() {
        let now = Utc::now();
        // A below-threshold score (< 0.5 for the `test` model) is not visible.
        let feed = feed(vec![
            entry("visible", now - chrono::Duration::hours(1), 0.9),
            entry("hidden", now - chrono::Duration::hours(1), 0.1),
        ]);
        let pairs = published_for_spec(
            &feed,
            Order::Recency,
            Limit::hours(48),
            &CdnImageUrlResolver,
            now,
        );
        assert_eq!(skeet_rkeys(&pairs), ["visible"]);
    }

    #[test]
    fn drops_images_that_cannot_be_resolved() {
        let now = Utc::now();
        // A V2 id has no recoverable cid, so the CDN resolver returns None.
        let mut bad = entry("v2", now - chrono::Duration::hours(1), 0.9);
        bad.0.image_id = "v2:0123456789abcdef0123456789abcdef"
            .parse()
            .expect("valid v2 id");
        let feed = feed(vec![
            entry("ok", now - chrono::Duration::hours(1), 0.9),
            bad,
        ]);
        let pairs = published_for_spec(
            &feed,
            Order::Recency,
            Limit::hours(48),
            &CdnImageUrlResolver,
            now,
        );
        assert_eq!(skeet_rkeys(&pairs), ["ok"]);
    }

    #[test]
    fn manual_band_override_hides_skeet() {
        let now = Utc::now();
        let e = entry("demoted", now - chrono::Duration::hours(1), 0.9);
        let demoted_skeet = e.0.skeet_id.clone();
        let mut feed = feed(vec![e]);
        feed.skeet_appraisals.insert(
            demoted_skeet,
            Appraisal {
                band: Band::Low,
                appraiser: Appraiser::LocalAdmin,
            },
        );
        let pairs = published_for_spec(
            &feed,
            Order::Recency,
            Limit::hours(48),
            &CdnImageUrlResolver,
            now,
        );
        assert!(pairs.is_empty());
    }
}
