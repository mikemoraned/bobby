#![warn(clippy::all, clippy::nursery)]

pub mod feed_config;
pub mod feed_source;
pub mod handlers;
pub mod project;
pub mod published_images_source;
pub mod qr;

pub use feed_source::{FeedSourceExtractor, FeedSourceLayer};
pub use published_images_source::{PublishedImagesSourceExtractor, PublishedImagesSourceLayer};

/// The one canonical description of the feed, shared by the Bluesky feed
/// registration (`register-feed`'s `--description`) and the website banner so
/// the two can't drift.
pub const FEED_BLURB: &str = "Selfies people take with landmarks — famous buildings, monuments and places — found on Bluesky.";
