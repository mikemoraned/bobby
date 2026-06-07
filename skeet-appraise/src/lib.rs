#![warn(clippy::all, clippy::nursery)]

pub mod admin;
pub mod appraiser_config;
pub mod auth;
pub mod auth_config;
pub mod available_feeds;
pub mod feed_snapshot;
pub mod handlers;
mod models_middleware;
pub mod project;
pub mod published_feed_middleware;
pub mod started_at;
pub mod static_assets;
mod store_middleware;

pub use appraiser_config::{AppraiserExtractor, AppraiserLayer};
pub use auth_config::{OAuthConfigExtractor, OAuthConfigLayer};
pub use models_middleware::{Models, ModelsLayer};
pub use published_feed_middleware::PublishedFeedLayer;
pub use started_at::{StartedAtExtractor, StartedAtLayer};
pub use static_assets::web_static_files;
pub use store_middleware::{Store, StoreLayer};
