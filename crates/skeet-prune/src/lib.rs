#![warn(clippy::all, clippy::nursery)]

mod classify;
mod firehose;
mod metrics;
mod persistence;
mod pipeline;
mod status;

pub use classify::classify;
pub use firehose::SkeetCandidate;
pub use pipeline::{
    ChannelMonitors, ImageResult, MetaResult, PipelineCounters, firehose_stage,
    prune_image_stage, prune_meta_stage, save_stage,
};
