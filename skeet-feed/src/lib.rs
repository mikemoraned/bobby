#![warn(clippy::all, clippy::nursery)]

pub mod admin;
pub mod appraiser_config;
pub mod auth;
pub mod auth_config;
pub mod effective_band;
pub mod feed_cache;
pub mod feed_config;
pub mod handlers;
pub mod project;
pub mod started_at;
pub mod static_assets;
mod store_middleware;

pub use appraiser_config::{AppraiserExtractor, AppraiserLayer};
pub use auth_config::{OAuthConfigExtractor, OAuthConfigLayer};
pub use feed_cache::{FeedCache, FeedCacheExtractor, FeedCacheLayer};
pub use started_at::{StartedAtExtractor, StartedAtLayer};
pub use static_assets::web_static_files;
pub use store_middleware::{Store, StoreLayer};
