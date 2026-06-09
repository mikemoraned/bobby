use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use shared::skeet_id::SkeetId;

use crate::dimensions::Dimensions;
use crate::image_url::ImageUrl;

mod image_prober;
mod skeet_prober;

pub use image_prober::{CdnImageProber, ImageProber};
pub use skeet_prober::{CdnSkeetProber, SkeetProber};

/// Run `probe` over each item with at most `concurrency` futures in flight,
/// collecting the `(key, value)` each yields into a map. A panicked probe simply
/// drops its item from the result.
async fn probe_bounded<K, V, Fut>(
    items: &[K],
    concurrency: usize,
    probe: impl Fn(K) -> Fut + Send,
) -> HashMap<K, V>
where
    K: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
    V: Send + 'static,
    Fut: std::future::Future<Output = (K, V)> + Send + 'static,
{
    let concurrency = concurrency.max(1);
    let mut out = HashMap::with_capacity(items.len());
    let mut pending = items.iter().cloned();
    let mut in_flight = tokio::task::JoinSet::new();

    for item in pending.by_ref().take(concurrency) {
        in_flight.spawn(probe(item));
    }
    while let Some(result) = in_flight.join_next().await {
        if let Ok((key, value)) = result {
            out.insert(key, value);
        }
        if let Some(item) = pending.next() {
            in_flight.spawn(probe(item));
        }
    }
    out
}

/// Whether an image url still exists and, when known, its pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageStatus {
    pub exists: bool,
    pub dimensions: Option<Dimensions>,
}

/// The existence verdict for a batch of `(skeet, image-url)` pairs: whether each
/// skeet still exists and the [`ImageStatus`] of each image url.
#[derive(Debug, Default)]
pub struct ExistenceResults {
    pub skeets: HashMap<SkeetId, bool>,
    pub images: HashMap<ImageUrl, ImageStatus>,
}

/// Checks whether published skeets/images still exist on Bluesky.
#[async_trait]
pub trait ExistenceChecker: Send + Sync {
    /// For each unique skeet and image url in `items`, return whether it still
    /// exists (plus, for images, its dimensions).
    async fn check(&self, items: &[(SkeetId, ImageUrl)]) -> ExistenceResults;
}

struct SkeetEntry {
    exists: bool,
    checked_at: Instant,
}

struct ImageEntry {
    exists: bool,
    dimensions: Option<Dimensions>,
    checked_at: Instant,
}

/// An [`ExistenceChecker`] that probes Bluesky's and caches each
/// result for a TTL.
///
/// Only entries whose last check is older than the TTL are re-probed, bounding
/// the external call volume across frequent republishes; a deletion is therefore
/// seen at most one TTL late. Image dimensions never change for a CDN url, so
/// once measured they are retained even when a later re-probe can't read them.
pub struct CdnExistenceChecker {
    skeet_prober: Arc<dyn SkeetProber>,
    image_prober: Arc<dyn ImageProber>,
    ttl: Duration,
    skeets: Mutex<HashMap<SkeetId, SkeetEntry>>,
    images: Mutex<HashMap<ImageUrl, ImageEntry>>,
}

impl CdnExistenceChecker {
    /// A checker probing the real Bluesky CDN/API, caching for `ttl` and probing
    /// at most `concurrency` skeets / images at once.
    pub fn new(ttl: Duration, concurrency: usize) -> Self {
        Self::with_probers(
            Arc::new(CdnSkeetProber::new(concurrency)),
            Arc::new(CdnImageProber::new(concurrency)),
            ttl,
        )
    }

    fn with_probers(
        skeet_prober: Arc<dyn SkeetProber>,
        image_prober: Arc<dyn ImageProber>,
        ttl: Duration,
    ) -> Self {
        Self {
            skeet_prober,
            image_prober,
            ttl,
            skeets: Mutex::new(HashMap::new()),
            images: Mutex::new(HashMap::new()),
        }
    }

    /// Of `items`, those whose cached entry is missing or older than the TTL, so
    /// they need a fresh probe. `checked_at` reads each entry's last-check time.
    /// A poisoned lock (unlikely) means the cache can't be trusted, so every item is
    /// treated as stale.
    fn stale<K, V>(
        &self,
        cache: &Mutex<HashMap<K, V>>,
        items: &[K],
        checked_at: impl Fn(&V) -> Instant,
    ) -> Vec<K>
    where
        K: Clone + Eq + std::hash::Hash,
    {
        let cache = match cache.lock() {
            Ok(cache) => cache,
            Err(_poisoned) => return items.to_vec(),
        };
        let now = Instant::now();
        items
            .iter()
            .filter(|item| {
                cache
                    .get(*item)
                    .is_none_or(|entry| now.duration_since(checked_at(entry)) >= self.ttl)
            })
            .cloned()
            .collect()
    }

    async fn resolve_skeets(&self, skeets: &[SkeetId]) -> HashMap<SkeetId, bool> {
        let stale = self.stale(&self.skeets, skeets, |e| e.checked_at);
        if !stale.is_empty() {
            let probed = self.skeet_prober.probe_skeets(&stale).await;
            let now = Instant::now();
            if let Ok(mut cache) = self.skeets.lock() {
                for (skeet, exists) in probed {
                    cache.insert(
                        skeet,
                        SkeetEntry {
                            exists,
                            checked_at: now,
                        },
                    );
                }
            }
        }
        let cache = self.skeets.lock();
        skeets
            .iter()
            .map(|s| {
                let exists = cache
                    .as_ref()
                    .ok()
                    .and_then(|c| c.get(s))
                    .is_none_or(|e| e.exists);
                (s.clone(), exists)
            })
            .collect()
    }

    async fn resolve_images(&self, urls: &[ImageUrl]) -> HashMap<ImageUrl, ImageStatus> {
        let stale = self.stale(&self.images, urls, |e| e.checked_at);
        if !stale.is_empty() {
            let probed = self.image_prober.probe_images(&stale).await;
            let now = Instant::now();
            if let Ok(mut cache) = self.images.lock() {
                for (url, status) in probed {
                    // Dimensions are immutable for a CDN url, so a probe that
                    // couldn't read them keeps any earlier measurement.
                    let dimensions = status
                        .dimensions
                        .or_else(|| cache.get(&url).and_then(|e| e.dimensions));
                    cache.insert(
                        url,
                        ImageEntry {
                            exists: status.exists,
                            dimensions,
                            checked_at: now,
                        },
                    );
                }
            }
        }
        let cache = self.images.lock();
        urls.iter()
            .map(|u| {
                let status = cache.as_ref().ok().and_then(|c| c.get(u)).map_or(
                    ImageStatus {
                        exists: true,
                        dimensions: None,
                    },
                    |e| ImageStatus {
                        exists: e.exists,
                        dimensions: e.dimensions,
                    },
                );
                (u.clone(), status)
            })
            .collect()
    }
}

#[async_trait]
impl ExistenceChecker for CdnExistenceChecker {
    async fn check(&self, items: &[(SkeetId, ImageUrl)]) -> ExistenceResults {
        let skeets = unique(items.iter().map(|(s, _)| s.clone()));
        let urls = unique(items.iter().map(|(_, u)| u.clone()));
        ExistenceResults {
            skeets: self.resolve_skeets(&skeets).await,
            images: self.resolve_images(&urls).await,
        }
    }
}

/// A no-network [`ExistenceChecker`] for tests and local single-shot runs: every
/// skeet/image is present unless listed as missing, with optional preset
/// dimensions. Never touches the network.
pub struct StaticExistenceChecker {
    missing_skeets: HashSet<SkeetId>,
    missing_images: HashSet<ImageUrl>,
    dimensions: HashMap<ImageUrl, Dimensions>,
}

impl StaticExistenceChecker {
    pub fn all_present() -> Self {
        Self {
            missing_skeets: HashSet::new(),
            missing_images: HashSet::new(),
            dimensions: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_missing_skeets(mut self, skeets: impl IntoIterator<Item = SkeetId>) -> Self {
        self.missing_skeets.extend(skeets);
        self
    }

    #[must_use]
    pub fn with_missing_images(mut self, urls: impl IntoIterator<Item = ImageUrl>) -> Self {
        self.missing_images.extend(urls);
        self
    }

    #[must_use]
    pub fn with_dimensions(mut self, url: ImageUrl, dimensions: Dimensions) -> Self {
        self.dimensions.insert(url, dimensions);
        self
    }
}

#[async_trait]
impl ExistenceChecker for StaticExistenceChecker {
    async fn check(&self, items: &[(SkeetId, ImageUrl)]) -> ExistenceResults {
        let mut skeets = HashMap::new();
        let mut images = HashMap::new();
        for (skeet, url) in items {
            skeets.insert(skeet.clone(), !self.missing_skeets.contains(skeet));
            images.insert(
                url.clone(),
                ImageStatus {
                    exists: !self.missing_images.contains(url),
                    dimensions: self.dimensions.get(url).copied(),
                },
            );
        }
        ExistenceResults { skeets, images }
    }
}

/// The items in order, with duplicates removed.
fn unique<T: Clone + Eq + std::hash::Hash>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn skeet(rkey: &str) -> SkeetId {
        format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse()
            .expect("valid skeet id")
    }

    fn url(n: u32) -> ImageUrl {
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/cid{n}@jpeg"
        ))
        .expect("valid url")
    }

    /// A prober pair that records how many skeets/images it probed and replies
    /// that everything exists. Image dimensions are returned only on the *first*
    /// round so the merge-retains-dimensions behaviour can be observed.
    #[derive(Default)]
    struct RecordingProber {
        skeet_probes: AtomicUsize,
        image_rounds: AtomicUsize,
    }

    #[async_trait]
    impl SkeetProber for RecordingProber {
        async fn probe_skeets(&self, skeets: &[SkeetId]) -> HashMap<SkeetId, bool> {
            self.skeet_probes.fetch_add(skeets.len(), Ordering::Relaxed);
            skeets.iter().map(|s| (s.clone(), true)).collect()
        }
    }

    #[async_trait]
    impl ImageProber for RecordingProber {
        async fn probe_images(&self, urls: &[ImageUrl]) -> HashMap<ImageUrl, ImageStatus> {
            let first_round = self.image_rounds.fetch_add(1, Ordering::Relaxed) == 0;
            urls.iter()
                .map(|u| {
                    let status = ImageStatus {
                        exists: first_round,
                        dimensions: first_round.then_some(Dimensions {
                            width: 100,
                            height: 200,
                        }),
                    };
                    (u.clone(), status)
                })
                .collect()
        }
    }

    fn checker(prober: &Arc<RecordingProber>, ttl: Duration) -> CdnExistenceChecker {
        CdnExistenceChecker::with_probers(
            Arc::clone(prober) as Arc<dyn SkeetProber>,
            Arc::clone(prober) as Arc<dyn ImageProber>,
            ttl,
        )
    }

    #[tokio::test]
    async fn within_ttl_does_not_reprobe() {
        let prober = Arc::new(RecordingProber::default());
        let checker = checker(&prober, Duration::from_secs(3600));
        let items = vec![(skeet("a"), url(1))];

        checker.check(&items).await;
        checker.check(&items).await;

        assert_eq!(prober.skeet_probes.load(Ordering::Relaxed), 1);
        assert_eq!(prober.image_rounds.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn expired_ttl_reprobes() {
        let prober = Arc::new(RecordingProber::default());
        let checker = checker(&prober, Duration::ZERO);
        let items = vec![(skeet("a"), url(1))];

        checker.check(&items).await;
        checker.check(&items).await;

        assert_eq!(prober.skeet_probes.load(Ordering::Relaxed), 2);
        assert_eq!(prober.image_rounds.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn reprobe_without_dimensions_retains_earlier_measurement() {
        let prober = Arc::new(RecordingProber::default());
        let checker = checker(&prober, Duration::ZERO);
        let items = vec![(skeet("a"), url(1))];

        let first = checker.check(&items).await;
        assert_eq!(
            first.images[&url(1)],
            ImageStatus {
                exists: true,
                dimensions: Some(Dimensions {
                    width: 100,
                    height: 200
                }),
            }
        );

        // Second round reports no dimensions and exists=false; dims are kept.
        let second = checker.check(&items).await;
        assert_eq!(
            second.images[&url(1)],
            ImageStatus {
                exists: false,
                dimensions: Some(Dimensions {
                    width: 100,
                    height: 200
                }),
            }
        );
    }

    #[tokio::test]
    async fn static_checker_marks_listed_items_missing() {
        let checker = StaticExistenceChecker::all_present()
            .with_missing_skeets([skeet("gone")])
            .with_missing_images([url(2)])
            .with_dimensions(
                url(1),
                Dimensions {
                    width: 10,
                    height: 20,
                },
            );
        let items = vec![(skeet("here"), url(1)), (skeet("gone"), url(2))];

        let results = checker.check(&items).await;
        assert!(results.skeets[&skeet("here")]);
        assert!(!results.skeets[&skeet("gone")]);
        assert_eq!(
            results.images[&url(1)],
            ImageStatus {
                exists: true,
                dimensions: Some(Dimensions {
                    width: 10,
                    height: 20
                })
            }
        );
        assert!(!results.images[&url(2)].exists);
    }
}
