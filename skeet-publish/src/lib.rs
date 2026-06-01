#![warn(clippy::all, clippy::nursery)]

pub mod effective_band;
pub mod feed_cache;
pub mod image_url;
pub mod image_url_resolver;
pub mod limit;
pub mod order;
pub mod published_list;
pub mod published_pair;
pub mod publisher;
pub mod redis_client;
pub mod source;
pub mod visibility;

pub use feed_cache::{CachedFeed, FeedCache, RefreshOutcome};
pub use image_url::{ImageUrl, InvalidImageUrl};
pub use image_url_resolver::{CdnImageUrlResolver, ImageUrlResolver};
pub use limit::{InvalidLimit, Limit};
pub use order::{InvalidOrder, Order};
pub use published_list::{PublishedList, PublishedListError};
pub use published_pair::PublishedPair;
pub use publisher::{FeedPublisher, PublishError, WindowedFeed, pairs_for_spec};
pub use redis_client::connect;
pub use source::{FeedSkeleton, FeedSource, FeedSourceError, LiveFeedSource, RedisFeedSource};
pub use visibility::{FeedData, visible_entries};
