#![warn(clippy::all, clippy::nursery)]

mod classify;
pub mod content_filter;
pub mod firehose;
pub mod metadata;

pub use classify::{classify, classify_image};
