#![warn(clippy::all, clippy::nursery)]

pub mod feed_entry;
pub mod layout;
pub mod static_assets;
mod store_middleware;

pub use feed_entry::{FeedEntry, to_feed_entry};
pub use layout::BaseLayout;
pub use static_assets::web_static_files;
pub use store_middleware::{Store, StoreLayer};
