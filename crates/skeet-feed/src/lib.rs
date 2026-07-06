#![warn(clippy::all, clippy::nursery)]

mod feed_config;
mod feed_source;
mod handlers;
pub mod preview;
mod project;
mod published_images_source;
mod qr;

pub use feed_config::{FeedConfigLayer, FeedParams, FeedParamsError};
pub use feed_source::{FeedSourceExtractor, FeedSourceLayer};
pub use project::FeedProject;
pub use published_images_source::{PublishedImagesSourceExtractor, PublishedImagesSourceLayer};

/// The one canonical description of the feed, shared by the Bluesky feed
/// registration (`register-feed`'s `--description`) and the website banner so
/// the two can't drift.
pub const FEED_BLURB: &str = "Selfies people take with landmarks — famous buildings, monuments and places — found on Bluesky.";

/// The site's title, shared by the HTML `<title>` and the `og:title` meta tag so
/// the browser tab and a shared link's unfurl can't drift.
pub const SITE_TITLE: &str = "Bobby — selfies with landmarks";
