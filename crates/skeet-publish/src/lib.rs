#![warn(clippy::all, clippy::nursery)]

mod effective_band;
mod fallback;
mod image_url_resolver;
mod limit;
mod list_statistics;
mod metrics;
mod order;
mod prediction;
mod published;
mod published_list;
mod published_list_catalog;
mod publisher;
mod redis_client;
mod source;
mod spec;
mod table_watch;
mod visibility;

pub use effective_band::{image_effective_band, skeet_effective_band};
pub use fallback::{FallbackFeedSource, fallback_chain};
pub use image_url_resolver::{CdnImageUrlResolver, ImageUrlResolver};
pub use limit::{InvalidLimit, Limit};
pub use list_statistics::ListStatistics;
pub use metrics::PublishMetrics;
pub use order::{InvalidOrder, Order};
pub use prediction::{NextMatchPrediction, predict_next_match};
pub use published::PublishedImage;
pub use published_list::{InvalidListName, PublishedList, PublishedListError};
pub use published_list_catalog::PublishedListCatalog;
pub use publisher::{
    FeedPublisher, PublishError, PublishOutcome, WindowedFeed, published_for_spec,
};
pub use redis_client::connect;
pub use source::{
    FeedSkeleton, FeedSource, FeedSourceError, PublishedImages, PublishedImagesSource,
    RedisFeedSource,
};
pub use spec::{InvalidSpec, parse_spec};
pub use visibility::FeedData;
