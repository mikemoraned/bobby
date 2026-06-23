use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use shared::{Rejection, RejectionCategory};
use skeet_store::ImageRecord;
use tokio::sync::mpsc;

use crate::firehose::SkeetCandidate;

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
#[derive(Default)]
pub struct ContentCounts {
    pub posts: u64,
    pub images: u64,
    pub saved: u64,
}

/// Cumulative rejection counts broken down by reason, by detection category, and
/// by the category that was the sole detection.
#[derive(Default)]
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
    firehose_tx: mpsc::Sender<SkeetCandidate>,
    meta_tx: mpsc::Sender<MetaResult>,
    image_tx: mpsc::Sender<ImageResult>,
}

impl ChannelMonitors {
    pub const fn new(
        firehose_tx: mpsc::Sender<SkeetCandidate>,
        meta_tx: mpsc::Sender<MetaResult>,
        image_tx: mpsc::Sender<ImageResult>,
    ) -> Self {
        Self {
            firehose_tx,
            meta_tx,
            image_tx,
        }
    }

    fn depth(tx: &impl ChannelDepth) -> usize {
        tx.max_capacity() - tx.capacity()
    }

    pub fn firehose_depth(&self) -> usize {
        Self::depth(&self.firehose_tx)
    }

    pub fn meta_depth(&self) -> usize {
        Self::depth(&self.meta_tx)
    }

    pub fn image_depth(&self) -> usize {
        Self::depth(&self.image_tx)
    }
}

trait ChannelDepth {
    fn capacity(&self) -> usize;
    fn max_capacity(&self) -> usize;
}

impl<T> ChannelDepth for mpsc::Sender<T> {
    fn capacity(&self) -> usize {
        self.capacity()
    }

    fn max_capacity(&self) -> usize {
        self.max_capacity()
    }
}
