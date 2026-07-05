#![warn(clippy::all, clippy::nursery)]

//! A thin client for the public (unauthenticated) Bluesky AppView, plus
//! Jetstream ingress.
//!
//! Covers the AppView parts Bobby needs and the interpretation of their
//! responses (moderation labels, post availability), so the firehose pruner and
//! the feed publisher apply the same notion of "viewable post". [`firehose`] adds
//! the Jetstream side: connecting to the event stream and interpreting the
//! `app.bsky.feed.post` records it carries.

pub mod dimensions;
pub mod existence;
pub mod firehose;
pub mod image_url;
mod post_thread;

pub use dimensions::Dimensions;
pub use existence::{
    CdnExistenceChecker, ExistenceChecker, ExistenceResults, ImageStatus, StaticExistenceChecker,
};
pub use image_url::{ImageUrl, InvalidImageUrl, bsky_cdn_thumbnail_url};
pub use post_thread::{BlueskyError, blocked_labels, fetch_post_thread, post_is_available};
