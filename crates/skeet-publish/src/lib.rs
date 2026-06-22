#![warn(clippy::all, clippy::nursery)]

pub mod effective_band;
pub mod examined_count;
pub mod image_url_resolver;
pub mod limit;
pub mod metrics;
pub mod order;
pub mod published;
pub mod published_list;
pub mod published_list_catalog;
pub mod publisher;
pub mod redis_client;
pub mod source;
pub mod spec;
pub mod table_watch;
pub mod visibility;

pub use examined_count::ExaminedCount;
pub use image_url_resolver::{CdnImageUrlResolver, ImageUrlResolver};
pub use limit::{InvalidLimit, Limit};
pub use metrics::PublishMetrics;
pub use order::{InvalidOrder, Order};
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
