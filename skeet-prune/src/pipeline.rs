use std::sync::atomic::{AtomicU64, Ordering};

use shared::Rejection;
use skeet_store::ImageRecord;
use tokio::sync::mpsc;

use crate::firehose::SkeetCandidate;

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
