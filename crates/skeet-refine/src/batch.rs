//! `Batch` of (id, image) candidates for refinement, with completion bookkeeping.
//!
//! A batch's lifecycle is: build → [`Self::score_with`] → consume via
//! `PollingBatchSource::commit`. `score_with` drains the images, runs the
//! scorer concurrently, marks successes [`Self::mark_completed`], and returns
//! the outcomes for the caller to log and persist.

use std::collections::{HashMap, HashSet};

use futures::stream::{self, StreamExt};
use image::DynamicImage;
use shared::DiscoveredAt;
use shared::ImageId;
use shared::Score;
use skeet_store::StoredOriginal;

#[derive(Default)]
pub struct Batch {
    pub ids: Vec<ImageId>,
    pub images: Vec<DynamicImage>,
    discovered_at_by_id: HashMap<ImageId, DiscoveredAt>,
    completed: HashSet<ImageId>,
}

/// Outcome of scoring a batch — successes and failures, paired with their ids.
pub struct ScoreOutcomes<E> {
    pub successes: Vec<(ImageId, Score)>,
    pub failures: Vec<(ImageId, E)>,
}

impl Batch {
    pub const fn len(&self) -> usize {
        self.ids.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Discover-time timestamp for `id`, if it's part of this batch.
    pub fn discovered_at(&self, id: &ImageId) -> Option<&DiscoveredAt> {
        self.discovered_at_by_id.get(id)
    }

    /// Mark `id` as successfully processed.
    pub fn mark_completed(&mut self, id: &ImageId) {
        self.completed.insert(id.clone());
    }

    /// Watermark for the next tick, derived from completion bookkeeping:
    /// * if any member is uncompleted → the *oldest* uncompleted `discovered_at`
    ///   (so the next inclusive `>=` scan re-fetches that straggler)
    /// * else if every member completed → the newest `discovered_at` in the batch
    ///   (already-scored items at the boundary are weeded out by the unscored join)
    /// * `None` when the batch was empty
    pub(crate) fn watermark(&self) -> Option<DiscoveredAt> {
        if self.discovered_at_by_id.is_empty() {
            return None;
        }
        let oldest_uncompleted = self
            .discovered_at_by_id
            .iter()
            .filter(|(id, _)| !self.completed.contains(*id))
            .map(|(_, dt)| dt.clone())
            .min();
        oldest_uncompleted.or_else(|| self.discovered_at_by_id.values().max().cloned())
    }

    /// Score every image concurrently. Each spawned future carries its own
    /// id, so out-of-order completion can't mis-attribute results. Successes
    /// are marked completed in place; the caller logs and persists outcomes.
    pub async fn score_with<F, Fut, E>(&mut self, concurrency: usize, scorer: F) -> ScoreOutcomes<E>
    where
        F: Fn(DynamicImage) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<Score, E>> + Send,
        E: Send,
    {
        let items: Vec<(ImageId, DynamicImage)> = self
            .ids
            .iter()
            .cloned()
            .zip(self.images.drain(..))
            .collect();

        let mut results: HashMap<ImageId, Result<Score, E>> = stream::iter(items)
            .map(|(id, img)| {
                let fut = scorer(img);
                async move { (id, fut.await) }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        let mut successes = Vec::new();
        let mut failures = Vec::new();
        let ids = self.ids.clone();
        for id in ids {
            match results.remove(&id) {
                Some(Ok(score)) => {
                    self.mark_completed(&id);
                    successes.push((id, score));
                }
                Some(Err(e)) => failures.push((id, e)),
                None => {} // duplicate id; defensive
            }
        }
        ScoreOutcomes {
            successes,
            failures,
        }
    }
}

impl From<Vec<StoredOriginal>> for Batch {
    fn from(originals: Vec<StoredOriginal>) -> Self {
        let mut ids = Vec::with_capacity(originals.len());
        let mut images = Vec::with_capacity(originals.len());
        let mut discovered_at_by_id = HashMap::with_capacity(originals.len());
        for s in originals {
            discovered_at_by_id.insert(s.summary.image_id.clone(), s.summary.discovered_at);
            ids.push(s.summary.image_id);
            images.push(s.image);
        }
        Self {
            ids,
            images,
            discovered_at_by_id,
            completed: HashSet::new(),
        }
    }
}

#[cfg(test)]
impl Batch {
    /// Test helper: build a Batch with one (id, discovered_at) entry — no
    /// images. Used to exercise watermark/commit without going through a fetch.
    pub(crate) fn with_entry(id: ImageId, discovered_at: DiscoveredAt) -> Self {
        let mut b = Self::default();
        b.discovered_at_by_id.insert(id, discovered_at);
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use proptest::prelude::*;
    use test_support::{marker_image, marker_of, score_for};

    /// `score_with` is generic over its scorer's error, so tests don't need
    /// `RefineError` — a unit error is enough.
    #[derive(Debug, PartialEq, Eq)]
    struct TestError;

    fn build_batch(markers: impl IntoIterator<Item = u32>) -> (Batch, Vec<u32>) {
        let mut batch = Batch::default();
        let mut ordered_markers = Vec::new();
        for m in markers {
            let img = marker_image(m);
            let id = ImageId::from_image(&img);
            batch.ids.push(id);
            batch.images.push(img);
            ordered_markers.push(m);
        }
        (batch, ordered_markers)
    }

    /// Reversed delays force completion order to be the inverse of submission
    /// order. Asserts every id is paired with the score the scorer actually
    /// returned for that id — independent of completion order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn each_id_receives_its_own_score_under_reversed_completion_order() {
        let n: u32 = 4;
        let (mut batch, markers) = build_batch(0..n);
        let ids = batch.ids.clone();

        let outcomes: ScoreOutcomes<TestError> = batch
            .score_with(n as usize, |img| async move {
                let m = marker_of(&img);
                let delay_ms = u64::from(n - m) * 25;
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                Ok(score_for(m))
            })
            .await;

        assert!(outcomes.failures.is_empty());
        let by_id: HashMap<ImageId, Score> = outcomes.successes.into_iter().collect();
        for (id, m) in ids.iter().zip(&markers) {
            let actual = by_id.get(id).expect("success for id");
            assert_eq!(
                *actual,
                score_for(*m),
                "id with marker {m} should receive its own score"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

        /// Random delays produce arbitrary completion orders. Every id must
        /// still receive the score the scorer returned for it.
        #[test]
        fn each_id_receives_its_own_score_under_arbitrary_completion_order(
            spec in prop::collection::hash_map(0u32..50u32, 0u64..15u64, 2..=6),
            concurrency in 2usize..=6,
        ) {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(concurrency)
                .enable_all()
                .build()
                .expect("build runtime");

            let pairs: Vec<(u32, Option<Score>)> = runtime.block_on(async move {
                let (mut batch, markers) = build_batch(spec.keys().copied());
                let ids = batch.ids.clone();
                let delays = spec.clone();
                let outcomes: ScoreOutcomes<TestError> = batch
                    .score_with(concurrency, move |img| {
                        let m = marker_of(&img);
                        let d = *delays.get(&m).expect("known marker");
                        async move {
                            tokio::time::sleep(Duration::from_millis(d)).await;
                            Ok(score_for(m))
                        }
                    })
                    .await;
                let by_id: HashMap<ImageId, Score> =
                    outcomes.successes.into_iter().collect();
                ids.into_iter()
                    .zip(markers)
                    .map(|(id, m)| (m, by_id.get(&id).copied()))
                    .collect()
            });

            for (m, score) in pairs {
                match score {
                    Some(s) => prop_assert_eq!(s, score_for(m)),
                    None => prop_assert!(false, "missing score for marker {}", m),
                }
            }
        }
    }
}
