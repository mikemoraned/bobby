#![warn(clippy::all, clippy::nursery)]

mod classify;
pub mod content_filter;
pub mod prune_image_stage;
pub mod prune_meta_stage;
pub mod firehose;
pub mod firehose_stage;
pub mod metadata;
pub mod persistence;
pub mod pipeline;
pub mod save_stage;
pub mod status;

pub use classify::{classify, classify_image};
