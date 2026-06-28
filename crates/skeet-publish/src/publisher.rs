use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bluesky::{ExistenceChecker, ExistenceResults, ImageUrl};
use chrono::{DateTime, Utc};
use deadpool_redis::redis;
use shared::{Appraisal, Band, ImageId, NormalizedScore, OriginalAt, RefineModels, SkeetId};
use skeet_store::{
    AppraisalsSource, ModelScore, ScoredSummary, ScoredView, Scores, Statistics, StoreError,
    TableVersions, Version, VersionedCache,
};
use tokio::sync::RwLock;

use crate::effective_band::{image_effective_band, image_normalized_score, skeet_effective_band};
use crate::examined_count::ExaminedCount;
use crate::image_url_resolver::ImageUrlResolver;
use crate::limit::Limit;
use crate::list_statistics::ListStatistics;
use crate::order::Order;
use crate::published::PublishedImage;
use crate::published_list::{PublishedList, PublishedListError};
use crate::table_watch::relevant;
use crate::visibility::FeedData;

/// The publisher's snapshot of scored skeets in a recency window, plus the
/// manual appraisals and models the visibility policy needs.
///
/// Assembled from an uncapped, recency-windowed store query, and implements
/// [`FeedData`] so the shared visibility policy runs over it.
pub struct WindowedFeed {
    pub entries: Vec<ScoredSummary>,
    pub skeet_appraisals: HashMap<SkeetId, Appraisal>,
    pub image_appraisals: HashMap<ImageId, Appraisal>,
    pub models: Arc<RefineModels>,
}

impl FeedData for WindowedFeed {
    fn entries(&self) -> &[ScoredSummary] {
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
pub fn published_for_spec<F: FeedData>(
    feed: &F,
    order: Order,
    limit: Limit,
    resolver: &dyn ImageUrlResolver,
    now: DateTime<Utc>,
) -> Vec<PublishedImage> {
    let cutoff_us = (now - limit.window()).timestamp_micros();

    let mut windowed: Vec<ScoredSummary> = feed
        .visible_entries()
        .into_iter()
        .filter(|entry| entry.summary.original_at.timestamp_micros() >= cutoff_us)
        .collect();

    match order {
        Order::Recency => windowed.sort_by_key(recency_rank),
        Order::Quality => windowed.sort_by_key(|entry| quality_rank(feed, entry)),
    }

    windowed
        .into_iter()
        .filter_map(|entry| {
            let summary = entry.summary;
            resolver
                .resolve(&summary.skeet_id, &summary.image_id)
                .map(|image_url| {
                    PublishedImage::unprobed(image_url, summary.image_id, summary.skeet_id)
                })
        })
        .collect()
}

/// Overwrite a pair's existence flags + dimensions from an existence check.
///
/// A skeet/url absent from `results` keeps the fail-open defaults set by
/// [`PublishedImage::unprobed`] (present, dimensions unknown).
fn enrich(pair: &mut PublishedImage, results: &ExistenceResults) {
    if let Some(&exists) = results.skeets.get(&pair.skeet_id) {
        pair.skeet_id_exists = exists;
    }
    if let Some(status) = results.images.get(&pair.image_url) {
        pair.image_url_exists = status.exists;
        pair.image_url_dimensions = status.dimensions;
    }
}

#[derive(Debug, PartialEq, Eq)]
struct QualityRank {
    band: Option<Band>,
    score: Option<NormalizedScore>,
    image_id: ImageId,
    skeet_id: SkeetId,
}

impl Ord for QualityRank {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare other→self on band then score so the higher one ranks as `Less` and
        // sorts first; then image-id, skeet-id ascending make equal ranks deterministic.
        other
            .band
            .cmp(&self.band)
            .then_with(|| other.score.cmp(&self.score))
            .then_with(|| self.image_id.cmp(&other.image_id))
            .then_with(|| self.skeet_id.cmp(&other.skeet_id))
    }
}

impl PartialOrd for QualityRank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RecencyRank {
    original_at: OriginalAt,
    image_id: ImageId,
    skeet_id: SkeetId,
}

impl Ord for RecencyRank {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare other→self on `original_at` so the newer one sorts first; then image-id,
        // skeet-id ascending make equal-timestamp entries deterministic.
        other
            .original_at
            .cmp(&self.original_at)
            .then_with(|| self.image_id.cmp(&other.image_id))
            .then_with(|| self.skeet_id.cmp(&other.skeet_id))
    }
}

impl PartialOrd for RecencyRank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn quality_rank<F: FeedData>(feed: &F, entry: &ScoredSummary) -> QualityRank {
    let ScoredSummary {
        summary,
        scored: ModelScore {
            score,
            model_version,
        },
    } = entry;
    let image_band = image_effective_band(
        *score,
        model_version,
        feed.models(),
        feed.image_band(&summary.image_id),
    );
    QualityRank {
        band: skeet_effective_band(feed.skeet_band(&summary.skeet_id), &[image_band]),
        score: image_normalized_score(*score, model_version, feed.models()),
        image_id: summary.image_id.clone(),
        skeet_id: summary.skeet_id.clone(),
    }
}

fn recency_rank(entry: &ScoredSummary) -> RecencyRank {
    let summary = &entry.summary;
    RecencyRank {
        original_at: summary.original_at.clone(),
        image_id: summary.image_id.clone(),
        skeet_id: summary.skeet_id.clone(),
    }
}

/// Of the images the pruner sees, roughly this percentage pass scoring and are
/// saved to the store.
const SAVE_RATE_PERCENT: f64 = 0.2;

/// Estimate how many images the pruner has processed from the number that were
/// scored and saved, by inverting the save rate (see [`SAVE_RATE_PERCENT`]).
fn estimate_processed(scored: usize) -> u64 {
    (scored as f64 * 100.0 / SAVE_RATE_PERCENT).round() as u64
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
pub struct FeedPublisher<S> {
    store: Arc<S>,
    models: Arc<RefineModels>,
    resolver: Arc<dyn ImageUrlResolver>,
    checker: Arc<dyn ExistenceChecker>,
    specs: Vec<(Order, Limit)>,
    /// Gates republishing on the relevant table versions at the last publish.
    last_relevant: RwLock<VersionedCache<HashSet<Version>, ()>>,
}

impl<S: ScoredView + AppraisalsSource + Scores + TableVersions + Statistics> FeedPublisher<S> {
    pub fn new(
        store: Arc<S>,
        models: Arc<RefineModels>,
        resolver: Arc<dyn ImageUrlResolver>,
        checker: Arc<dyn ExistenceChecker>,
        specs: Vec<(Order, Limit)>,
    ) -> Self {
        Self {
            store,
            models,
            resolver,
            checker,
            specs,
            last_relevant: RwLock::new(VersionedCache::new()),
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

        let known_versions = self.models.versions().cloned().collect();
        let skeet_src = self.store.skeet_appraisals();
        let image_src = self.store.image_appraisals();
        let (entries, skeet_appraisals, image_appraisals) = tokio::try_join!(
            self.store
                .list_scored_summaries_published_since(now - widest, &known_versions),
            skeet_src.list_all(),
            image_src.list_all(),
        )?;

        Ok(WindowedFeed {
            entries,
            skeet_appraisals: skeet_appraisals.into_iter().collect(),
            image_appraisals: image_appraisals.into_iter().collect(),
            models: Arc::clone(&self.models),
        })
    }

    /// Compute and atomically publish every spec's list to redis.
    ///
    /// Candidate pairs for all specs are computed first, then enriched in one
    /// existence check over their union (so a skeet/url shared by several specs
    /// is probed once) before each list is written.
    pub async fn publish<C>(&self, conn: &mut C, now: DateTime<Utc>) -> Result<(), PublishError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let feed = self.fetch(now).await?;
        let mut per_spec: Vec<((Order, Limit), Vec<PublishedImage>)> =
            Vec::with_capacity(self.specs.len());
        for (order, limit) in &self.specs {
            let pairs = published_for_spec(&feed, *order, *limit, self.resolver.as_ref(), now);
            per_spec.push(((*order, *limit), pairs));
        }

        let items: Vec<(SkeetId, ImageUrl)> = per_spec
            .iter()
            .flat_map(|(_, pairs)| {
                pairs
                    .iter()
                    .map(|p| (p.skeet_id.clone(), p.image_url.clone()))
            })
            .collect();
        let results = self.checker.check(&items).await;

        for ((order, limit), mut pairs) in per_spec {
            for pair in &mut pairs {
                enrich(pair, &results);
            }
            let list = PublishedList::new(order, limit);
            list.replace(conn, &pairs, now).await?;

            // Statistics cover the list's absolute window — examined over it, plus
            // how many we found to show (the list length).
            let interval_start = now - limit.window();
            let examined = self
                .store
                .interval_counts(interval_start, now)
                .await?
                .images_examined;
            let stats = ListStatistics::new(interval_start, now, examined, pairs.len() as u64);
            list.write_statistics(conn, &stats).await?;
        }

        let scored = self
            .store
            .count_scored_images(&self.models.versions().cloned().collect())
            .await?;
        ExaminedCount::write(conn, estimate_processed(scored)).await?;

        Ok(())
    }

    /// Publish if a relevant table version (scores or appraisals) has moved since
    /// the last publish, **or** if any target list is missing from redis — so an
    /// idle worker skips the full store fetch and redis writes when nothing has
    /// changed, yet still restores a list that was evicted/flushed/deleted
    /// out-of-band.
    pub async fn publish_if_changed<C>(
        &self,
        conn: &mut C,
        now: DateTime<Utc>,
    ) -> Result<PublishOutcome, PublishError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let snapshot = self.store.version_snapshot().await?;
        let key = relevant(&snapshot);
        let store_unchanged = self.last_relevant.read().await.is_cached_current(&key);
        if store_unchanged && self.all_lists_present(conn).await? {
            return Ok(PublishOutcome::Unchanged);
        }

        self.publish(conn, now).await?;
        self.last_relevant.write().await.cache(key, ());
        Ok(PublishOutcome::Published(self.specs.clone()))
    }

    /// Whether every configured list currently exists in redis.
    async fn all_lists_present<C>(&self, conn: &mut C) -> Result<bool, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        for (order, limit) in &self.specs {
            if !PublishedList::new(*order, *limit).exists(conn).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::refine_model::{ModelName, ModelProvider, RefinePrompt};
    use shared::{
        Appraiser, Band, BlueskyCid, DiscoveredAt, OriginalAt, RefineModel, Threshold, Zone,
    };
    use skeet_store::{ModelVersion, Score, StoredImageSummary};
    use test_support::test_models;

    use crate::image_url_resolver::CdnImageUrlResolver;

    const CID: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

    /// A scored entry for skeet `rkey`, published `published`, with a positive
    /// score (≥ 0.5 for the `test` model) and a `V3` image id so the CDN
    /// resolver succeeds.
    fn entry(rkey: &str, published: DateTime<Utc>, score: f32) -> ScoredSummary {
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
        ScoredSummary {
            summary,
            scored: ModelScore {
                score: Score::new(score).expect("valid score"),
                model_version: ModelVersion::from("test"),
            },
        }
    }

    fn feed(entries: Vec<ScoredSummary>) -> WindowedFeed {
        WindowedFeed {
            entries,
            skeet_appraisals: HashMap::new(),
            image_appraisals: HashMap::new(),
            models: test_models(),
        }
    }

    fn skeet_rkeys(pairs: &[PublishedImage]) -> Vec<String> {
        pairs
            .iter()
            .map(|p| p.skeet_id.rkey().as_str().to_string())
            .collect()
    }

    #[test]
    fn estimate_processed_inverts_the_save_rate() {
        // 0.2% save rate ⇒ multiply the saved count by 500.
        assert_eq!(estimate_processed(43243), 21_621_500);
        assert_eq!(estimate_processed(0), 0);
        assert_eq!(estimate_processed(1), 500);
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
        bad.summary.image_id = "v2:0123456789abcdef0123456789abcdef"
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
        let demoted_skeet = e.summary.skeet_id.clone();
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

    // ─── Quality ordering ───────────────────────────────────────────

    /// A registry with one model per `(version, decision_threshold)` spec.
    fn models_with(specs: &[(&str, f64)]) -> Arc<RefineModels> {
        let mut models = RefineModels::new();
        for (version, threshold) in specs {
            models.insert_unverified(
                version,
                RefineModel {
                    model_provider: ModelProvider::openai(),
                    model_name: ModelName::gpt_4o(),
                    prompt: RefinePrompt::new("test"),
                    decision_threshold: Threshold::new(*threshold).expect("valid"),
                },
            );
        }
        Arc::new(models)
    }

    /// A recent entry (inside any window) scored `score` by model `model`.
    fn entry_m(now: DateTime<Utc>, rkey: &str, score: f32, model: &str) -> ScoredSummary {
        let mut e = entry(rkey, now - chrono::Duration::hours(1), score);
        e.scored.model_version = ModelVersion::from(model);
        e
    }

    fn quality_feed(entries: Vec<ScoredSummary>, models: Arc<RefineModels>) -> WindowedFeed {
        WindowedFeed {
            entries,
            skeet_appraisals: HashMap::new(),
            image_appraisals: HashMap::new(),
            models,
        }
    }

    fn quality_rkeys(feed: &WindowedFeed, now: DateTime<Utc>) -> Vec<String> {
        let pairs = published_for_spec(
            feed,
            Order::Quality,
            Limit::hours(48),
            &CdnImageUrlResolver,
            now,
        );
        skeet_rkeys(&pairs)
    }

    #[test]
    fn quality_band_beats_score() {
        let now = Utc::now();
        // `lower` (lenient t=0.2) bands High at score 0.6; `higher` (t=0.5) bands
        // only MedHigh at the *higher* score 0.7 — band dominates raw score.
        let feed = quality_feed(
            vec![
                entry_m(now, "medhigh", 0.7, "t05"),
                entry_m(now, "high", 0.6, "t02"),
            ],
            models_with(&[("t02", 0.2), ("t05", 0.5)]),
        );
        assert_eq!(quality_rkeys(&feed, now), ["high", "medhigh"]);
    }

    #[test]
    fn quality_within_band_higher_score_first() {
        let now = Utc::now();
        // Both MedHigh under t=0.5; the higher score sorts first.
        let feed = quality_feed(
            vec![
                entry_m(now, "lo", 0.6, "t05"),
                entry_m(now, "hi", 0.7, "t05"),
            ],
            models_with(&[("t05", 0.5)]),
        );
        assert_eq!(quality_rkeys(&feed, now), ["hi", "lo"]);
    }

    #[test]
    fn quality_manual_band_reorders_relative_to_score() {
        let now = Utc::now();
        let demoted = entry_m(now, "demoted", 0.95, "t05");
        let plain = entry_m(now, "plain", 0.90, "t05");
        let demoted_skeet = demoted.summary.skeet_id.clone();

        // Score-alone both land in High, so the higher score (`demoted`, 0.95) leads.
        let feed = quality_feed(
            vec![demoted.clone(), plain.clone()],
            models_with(&[("t05", 0.5)]),
        );
        assert_eq!(quality_rkeys(&feed, now), ["demoted", "plain"]);

        // A manual MedHigh on `demoted` drops it a band below `plain` (still High),
        // flipping the order despite `demoted`'s higher score.
        let mut feed = quality_feed(vec![demoted, plain], models_with(&[("t05", 0.5)]));
        feed.skeet_appraisals.insert(
            demoted_skeet,
            Appraisal {
                band: Band::MediumHigh,
                appraiser: Appraiser::LocalAdmin,
            },
        );
        assert_eq!(quality_rkeys(&feed, now), ["plain", "demoted"]);
    }

    #[test]
    fn quality_drops_below_threshold_score_for_strict_model() {
        let now = Utc::now();
        // With t=0.6, a 0.55 score normalises below 0.5 → MedLow → hidden; 0.9 stays.
        let feed = quality_feed(
            vec![
                entry_m(now, "kept", 0.9, "t06"),
                entry_m(now, "below", 0.55, "t06"),
            ],
            models_with(&[("t06", 0.6)]),
        );
        assert_eq!(quality_rkeys(&feed, now), ["kept"]);
    }

    /// The appraise-website example. Two skeets that tie on the quality key — both
    /// effective `MedHigh` at score 0.95 (A capped by its manual skeet band, B by a
    /// manual image band). Their order must depend only on the data, not on the order
    /// the store returned them: the tie-break (image-id then skeet-id) makes the
    /// result identical under any input permutation.
    #[test]
    fn quality_appraise_example_orders_deterministically() {
        let now = Utc::now();
        // Distinct image ids so B's manual image override doesn't also hit A.
        const CID_B: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqkucnbc24deqo72b6mkl2egezxhvy";

        let mut a = entry_m(now, "a", 0.95, "t05");
        a.summary.image_id = ImageId::V3(BlueskyCid::new(CID).expect("valid cid"));
        let mut b = entry_m(now, "b", 0.95, "t05");
        b.summary.image_id = ImageId::V3(BlueskyCid::new(CID_B).expect("valid cid"));

        let a_skeet = a.summary.skeet_id.clone();
        let b_skeet = b.summary.skeet_id.clone();
        let b_image = b.summary.image_id.clone();

        let setup = |entries: Vec<ScoredSummary>| {
            let mut feed = quality_feed(entries, models_with(&[("t05", 0.5)]));
            for skeet in [a_skeet.clone(), b_skeet.clone()] {
                feed.skeet_appraisals.insert(
                    skeet,
                    Appraisal {
                        band: Band::MediumHigh,
                        appraiser: Appraiser::LocalAdmin,
                    },
                );
            }
            feed.image_appraisals.insert(
                b_image.clone(),
                Appraisal {
                    band: Band::MediumHigh,
                    appraiser: Appraiser::LocalAdmin,
                },
            );
            feed
        };

        let forward = quality_rkeys(&setup(vec![a.clone(), b.clone()]), now);
        let reversed = quality_rkeys(&setup(vec![b, a]), now);
        assert_eq!(
            forward, reversed,
            "quality order must not depend on input order"
        );
        // CID < CID_B, so the tie-break puts A first regardless of input order.
        assert_eq!(forward, ["a", "b"]);
    }

    #[test]
    fn quality_ties_are_deterministic_regardless_of_input_order() {
        let now = Utc::now();
        // Both High band, identical score → a pure tie broken only by id.
        let forward = quality_feed(
            vec![
                entry_m(now, "aaa", 0.9, "t05"),
                entry_m(now, "bbb", 0.9, "t05"),
            ],
            models_with(&[("t05", 0.5)]),
        );
        let reversed = quality_feed(
            vec![
                entry_m(now, "bbb", 0.9, "t05"),
                entry_m(now, "aaa", 0.9, "t05"),
            ],
            models_with(&[("t05", 0.5)]),
        );
        assert_eq!(quality_rkeys(&forward, now), quality_rkeys(&reversed, now));
        assert_eq!(quality_rkeys(&forward, now), ["aaa", "bbb"]);
    }

    #[test]
    fn recency_ties_are_deterministic_regardless_of_input_order() {
        let now = Utc::now();
        let t = now - chrono::Duration::hours(1);
        // Identical `original_at` → recency ties, broken only by id.
        let recency = |entries| {
            let feed = feed(entries);
            skeet_rkeys(&published_for_spec(
                &feed,
                Order::Recency,
                Limit::hours(48),
                &CdnImageUrlResolver,
                now,
            ))
        };
        let forward = recency(vec![entry("aaa", t, 0.9), entry("bbb", t, 0.9)]);
        let reversed = recency(vec![entry("bbb", t, 0.9), entry("aaa", t, 0.9)]);
        assert_eq!(
            forward, reversed,
            "recency order must not depend on input order"
        );
        assert_eq!(forward, ["aaa", "bbb"]);
    }

    #[test]
    fn quality_within_band_tiebreak_is_cross_model() {
        let now = Utc::now();
        // Same band (MedHigh), different thresholds: `a` (t=0.5, score 0.70) is
        // further past its threshold than `b` (t=0.6, score 0.72 → normalises 0.65),
        // so `a` leads even though its raw score is lower. Raw-score sort would
        // invert this — the regression guard for normalisation.
        let feed = quality_feed(
            vec![
                entry_m(now, "b", 0.72, "t06"),
                entry_m(now, "a", 0.70, "t05"),
            ],
            models_with(&[("t05", 0.5), ("t06", 0.6)]),
        );
        assert_eq!(quality_rkeys(&feed, now), ["a", "b"]);
    }
}
