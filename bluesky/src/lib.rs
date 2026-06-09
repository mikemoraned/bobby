#![warn(clippy::all, clippy::nursery)]

//! A thin client for the public (unauthenticated) Bluesky AppView.
//!
//! Covers the parts Bobby needs plus the interpretation of their responses
//! (moderation labels, post availability), so the firehose pruner and the feed
//! publisher apply the same notion of "viewable post".

pub mod dimensions;
pub mod existence;
pub mod image_url;
mod post_thread;

pub use dimensions::Dimensions;
pub use existence::{
    CdnExistenceChecker, ExistenceChecker, ExistenceResults, ImageStatus, StaticExistenceChecker,
};
pub use image_url::{ImageUrl, InvalidImageUrl};
pub use post_thread::{BlueskyError, blocked_labels, fetch_post_thread, post_is_available};
