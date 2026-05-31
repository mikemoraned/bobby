#![warn(clippy::all, clippy::nursery)]

pub mod feed_config;
pub mod feed_source;
pub mod handlers;
pub mod project;

pub use feed_source::{FeedSourceExtractor, FeedSourceLayer};
