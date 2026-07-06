#![warn(clippy::all, clippy::nursery)]

mod admin;
mod appraiser_config;
mod auth;
mod auth_config;
mod available_feeds;
mod feed_snapshot;
mod handlers;
mod models_middleware;
mod project;
mod published_feed_middleware;
mod started_at;
mod static_assets;
mod store_middleware;

pub use appraiser_config::{AppraiserExtractor, AppraiserLayer};
pub use auth_config::{OAuthConfig, OAuthConfigError, OAuthConfigExtractor, OAuthConfigLayer};
pub use available_feeds::PublishedListCatalogReader;
pub use models_middleware::{Models, ModelsLayer};
pub use project::AppraiseProject;
pub use published_feed_middleware::PublishedFeedLayer;
pub use started_at::{StartedAtExtractor, StartedAtLayer};
pub use static_assets::web_static_files;
pub use store_middleware::{AppraiseStore, Store, StoreLayer};
