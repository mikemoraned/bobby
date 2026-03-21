#![warn(clippy::all, clippy::nursery)]

mod classify;
pub mod content_filter;
pub mod firehose;
pub mod metadata;
pub mod persistence;
pub mod status;

pub use classify::{classify, classify_image};
