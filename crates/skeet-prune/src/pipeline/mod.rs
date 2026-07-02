//! The ordered pipeline stages — firehose → meta → image → save → stats — and
//! the message types, counters, and shutdown seam they share.
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::{Add, AddAssign};
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::{Receiver, Sender};
use shared::{Rejection, RejectionCategory};
use skeet_store::ImageRecord;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::firehose::SkeetCandidate;

pub mod content_counts_recorder;
pub mod content_statistics_stage;
pub mod firehose_stage;
pub mod prune_image_stage;
pub mod prune_meta_stage;
pub mod save_stage;
pub mod statistics_persister;

/// A stage should stop: either the downstream receiver was dropped or shutdown
/// was requested on the shared [`CancellationToken`].
pub struct Stopped;

/// Forward `item` to the next stage while observing the shared shutdown token.
///
/// A dropped downstream receiver is treated as a pipeline-wide shutdown: the
/// token is cancelled so every other stage unwinds through the same seam.
/// Returns `Err(Stopped)` when the caller should stop — either because
/// downstream is gone or because shutdown was already in progress.
pub async fn forward<T>(tx: &Sender<T>, item: T, token: &CancellationToken) -> Result<(), Stopped> {
    tokio::select! {
        () = token.cancelled() => Err(Stopped),
        sent = tx.send(item) => sent.map_err(|_| {
            warn!("downstream closed, shutting down pipeline");
            token.cancel();
            Stopped
        }),
    }
}

/// Receive the next item, or `None` once the channel is closed or shutdown was
/// requested on the shared [`CancellationToken`].
///
/// The receiver is a multi-consumer [`async_channel::Receiver`], so it is shared
/// by `&self` across a stage's worker pool rather than owned by one consumer.
pub async fn recv<T>(rx: &Receiver<T>, token: &CancellationToken) -> Option<T> {
    tokio::select! {
        () = token.cancelled() => None,
        item = rx.recv() => item.ok(),
    }
}

/// Cumulative throughput and current queue depth for one pipeline stage.
#[derive(Default)]
pub struct StageStats {
    pub throughput: u64,
    pub depth: usize,
}

/// Per-stage pipeline metrics.
#[derive(Default)]
pub struct PipelineStages {
    pub firehose: StageStats,
    pub meta: StageStats,
    pub image: StageStats,
}

/// The pipeline's content tallies: skeets seen, images examined and saved, and
/// the rejection breakdown. Each stage folds the work it did into the
/// `ContentCounts` it forwards, and the sink merges them all into one running
/// total.
///
/// A commutative monoid under `+` / `+=`: [`Default`] is the identity and the
/// combine is associative (saturating, so the laws hold for all `u64` without
/// overflow).
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct ContentCounts {
    pub posts: u64,
    pub images: u64,
    pub saved: u64,
    pub rejected: u64,
    pub rejections: RejectionBreakdown,
}

impl ContentCounts {
    /// The delta for one observed post that passed metadata: one skeet and its
    /// `images` examined.
    #[must_use]
    pub fn post(images: u64) -> Self {
        Self {
            posts: 1,
            images,
            ..Self::default()
        }
    }

    /// The delta for one image saved to the store.
    #[must_use]
    pub fn saved() -> Self {
        Self {
            saved: 1,
            ..Self::default()
        }
    }

    /// The delta for one rejected image: it bumps the headline count, each
    /// reason, each distinct detection category, and — when a single category
    /// was the sole detection — that category's sole tally.
    #[must_use]
    pub fn rejected(reasons: &[Rejection]) -> Self {
        let mut rejections = RejectionBreakdown::default();
        let mut categories_seen: HashSet<RejectionCategory> = HashSet::new();
        for reason in reasons {
            *rejections.by_reason.entry(*reason).or_default() += 1;
            categories_seen.insert(reason.category());
        }
        for &cat in &categories_seen {
            *rejections.by_category.entry(cat).or_default() += 1;
        }
        if categories_seen.len() == 1
            && let Some(sole) = categories_seen.into_iter().next()
        {
            *rejections.by_sole_category.entry(sole).or_default() += 1;
        }
        Self {
            rejected: 1,
            rejections,
            ..Self::default()
        }
    }
}

impl Add for ContentCounts {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self {
        self += &rhs;
        self
    }
}

impl AddAssign<&Self> for ContentCounts {
    fn add_assign(&mut self, rhs: &Self) {
        self.posts = self.posts.saturating_add(rhs.posts);
        self.images = self.images.saturating_add(rhs.images);
        self.saved = self.saved.saturating_add(rhs.saved);
        self.rejected = self.rejected.saturating_add(rhs.rejected);
        merge_counts(&mut self.rejections.by_reason, &rhs.rejections.by_reason);
        merge_counts(&mut self.rejections.by_category, &rhs.rejections.by_category);
        merge_counts(
            &mut self.rejections.by_sole_category,
            &rhs.rejections.by_sole_category,
        );
    }
}

/// Add every `from` entry into `into`, summing on key collision.
fn merge_counts<K: Eq + Hash + Clone>(into: &mut HashMap<K, u64>, from: &HashMap<K, u64>) {
    for (key, &count) in from {
        *into.entry(key.clone()).or_default() += count;
    }
}

/// Cumulative rejection counts broken down by reason, by detection category, and
/// by the category that was the sole detection.
#[derive(Default, Clone, PartialEq, Eq, Debug)]
pub struct RejectionBreakdown {
    pub by_reason: HashMap<Rejection, u64>,
    pub by_category: HashMap<RejectionCategory, u64>,
    pub by_sole_category: HashMap<RejectionCategory, u64>,
}

/// One status-interval's worth of pipeline numbers, captured once and handed to
/// each consumer (OTel today, the store's statistics record later). Carries no
/// telemetry-backend dependency so the same value can be reused by either.
#[derive(Default)]
pub struct PipelineSnapshot {
    pub stages: PipelineStages,
    pub content: ContentCounts,
}

/// Metadata-stage outcome for one candidate: either it needs image
/// classification, or it was rejected (the rejection is already tallied into the
/// accompanying [`ContentCounts`], so there's nothing left to carry).
pub enum MetaResult {
    Candidate(SkeetCandidate),
    Rejected,
}

/// A `prune_meta_stage` → `prune_image_stage` message: one candidate's metadata
/// outcome paired with the content-count delta the meta stage produced.
pub type MetaMessage = (MetaResult, ContentCounts);

/// A `prune_image_stage` → `save_stage` message.
///
/// The image records to persist (empty when nothing survived classification, or
/// for a metadata-rejected post) paired with the single content-count delta the
/// candidate contributes — rejections already folded in upstream.
pub type ImageMessage = (Vec<ImageRecord>, ContentCounts);

/// A `save_stage` → `content_statistics_stage` message.
///
/// One candidate's content-count delta with the save stage's `saved` decision
/// now folded in. The work half is exhausted by the time it reaches the final
/// stage, so only the tally rides on.
pub type StatsMessage = ContentCounts;

/// Per-stage item counters for throughput monitoring.
#[derive(Default)]
pub struct PipelineCounters {
    pub firehose: AtomicU64,
    pub meta: AtomicU64,
    pub image: AtomicU64,
}

impl PipelineCounters {
    pub fn firehose_count(&self) -> u64 {
        self.firehose.load(Ordering::Relaxed)
    }

    pub fn meta_count(&self) -> u64 {
        self.meta.load(Ordering::Relaxed)
    }

    pub fn image_count(&self) -> u64 {
        self.image.load(Ordering::Relaxed)
    }
}

/// Handles to monitor channel depths from the save stage.
pub struct ChannelMonitors {
    firehose_tx: Sender<SkeetCandidate>,
    meta_tx: Sender<MetaMessage>,
    image_tx: Sender<ImageMessage>,
}

impl ChannelMonitors {
    pub const fn new(
        firehose_tx: Sender<SkeetCandidate>,
        meta_tx: Sender<MetaMessage>,
        image_tx: Sender<ImageMessage>,
    ) -> Self {
        Self {
            firehose_tx,
            meta_tx,
            image_tx,
        }
    }

    pub fn firehose_depth(&self) -> usize {
        self.firehose_tx.len()
    }

    pub fn meta_depth(&self) -> usize {
        self.meta_tx.len()
    }

    pub fn image_depth(&self) -> usize {
        self.image_tx.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use shared::Rejection;

    prop_compose! {
        fn counts()(
            posts in any::<u64>(),
            images in any::<u64>(),
            saved in any::<u64>(),
            reasons in prop::collection::vec(
                prop_oneof![
                    Just(Rejection::FaceTooSmall),
                    Just(Rejection::TooMuchText),
                    Just(Rejection::BlockedByMetadata),
                ],
                0..4,
            ),
        ) -> ContentCounts {
            let mut c = ContentCounts { posts, images, saved, ..ContentCounts::default() };
            if !reasons.is_empty() {
                c += &ContentCounts::rejected(&reasons);
            }
            c
        }
    }

    proptest! {
        #[test]
        fn add_has_default_identity(c in counts()) {
            prop_assert_eq!(ContentCounts::default() + c.clone(), c.clone());
            prop_assert_eq!(c.clone() + ContentCounts::default(), c);
        }

        #[test]
        fn add_is_associative(a in counts(), b in counts(), c in counts()) {
            let left = (a.clone() + b.clone()) + c.clone();
            let right = a + (b + c);
            prop_assert_eq!(left, right);
        }
    }

    #[test]
    fn rejected_tallies_count_reasons_and_categories() {
        // Two reasons in the same category (Face): one rejected image, both
        // reasons, one Face category, and Face counts as the sole category.
        let c = ContentCounts::rejected(&[Rejection::FaceTooSmall, Rejection::FaceTooSmall]);
        assert_eq!(c.rejected, 1);
        assert_eq!(c.rejections.by_reason[&Rejection::FaceTooSmall], 2);
        assert_eq!(c.rejections.by_category[&RejectionCategory::Face], 1);
        assert_eq!(c.rejections.by_sole_category[&RejectionCategory::Face], 1);

        // Reasons spanning two categories: no sole category.
        let c = ContentCounts::rejected(&[Rejection::FaceTooSmall, Rejection::TooMuchText]);
        assert_eq!(c.rejected, 1);
        assert_eq!(c.rejections.by_category[&RejectionCategory::Face], 1);
        assert_eq!(c.rejections.by_category[&RejectionCategory::Text], 1);
        assert!(c.rejections.by_sole_category.is_empty());
    }
}
