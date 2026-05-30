#![warn(clippy::all, clippy::nursery)]

pub mod admin;
pub mod appraiser_config;
pub mod auth;
pub mod auth_config;
pub mod feed_cache_middleware;
pub mod feed_config;
pub mod feed_source;
pub mod handlers;
pub mod project;
pub mod started_at;
pub mod static_assets;
mod store_middleware;

pub use appraiser_config::{AppraiserExtractor, AppraiserLayer};
pub use auth_config::{OAuthConfigExtractor, OAuthConfigLayer};
pub use feed_cache_middleware::{FeedCacheExtractor, FeedCacheLayer};
pub use feed_source::{FeedSourceExtractor, FeedSourceLayer};
pub use started_at::{StartedAtExtractor, StartedAtLayer};
pub use static_assets::web_static_files;
pub use store_middleware::{Store, StoreLayer};
