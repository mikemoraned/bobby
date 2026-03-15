#![warn(clippy::all, clippy::nursery)]

mod classify;
pub mod content_filter;
pub mod firehose;

pub use classify::{classify, classify_image};
