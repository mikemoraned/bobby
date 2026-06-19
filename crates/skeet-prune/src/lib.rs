#![warn(clippy::all, clippy::nursery)]

mod classify;
pub mod firehose;
pub mod firehose_stage;
mod metrics;
pub mod persistence;
pub mod pipeline;
pub mod prune_image_stage;
pub mod prune_meta_stage;
pub mod save_stage;
pub mod status;

pub use classify::{classify, classify_image};
