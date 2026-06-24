//! The four ordered pipeline stages — firehose → meta → image → save — and the
//! message types, counters, and shutdown seam they share.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_channel::{Receiver, Sender};
use shared::{Rejection, RejectionCategory};
use skeet_store::ImageRecord;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::firehose::SkeetCandidate;

pub mod firehose_stage;
pub mod prune_image_stage;
pub mod prune_meta_stage;
pub mod save_stage;

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

/// Cumulative content counts: skeets seen, images examined, images saved.
#[derive(Default, Clone)]
pub struct ContentCounts {
    pub posts: u64,
    pub images: u64,
    pub saved: u64,
}

/// Cumulative rejection counts broken down by reason, by detection category, and
/// by the category that was the sole detection.
#[derive(Default, Clone)]
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
    pub rejections: RejectionBreakdown,
}

/// Messages between `prune_meta_stage` and `prune_image_stage`.
pub enum MetaResult {
    Post { image_count: u64 },
    Candidate(SkeetCandidate),
    Rejected(Vec<Rejection>),
}

/// Messages between `prune_image_stage` and `save_stage`.
pub enum ImageResult {
    Post { image_count: u64 },
    Classified(Box<ImageRecord>),
    Rejected(Vec<Rejection>),
}

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
    meta_tx: Sender<MetaResult>,
    image_tx: Sender<ImageResult>,
}

impl ChannelMonitors {
    pub const fn new(
        firehose_tx: Sender<SkeetCandidate>,
        meta_tx: Sender<MetaResult>,
        image_tx: Sender<ImageResult>,
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
