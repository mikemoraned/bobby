use shared::Rejection;
use skeet_store::ImageRecord;

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
