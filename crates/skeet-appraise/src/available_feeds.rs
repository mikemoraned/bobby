use std::collections::HashMap;
use std::sync::Arc;

use deadpool_redis::redis::RedisError;
use skeet_publish::{
    Limit, Order, PublishedList, PublishedListCatalog, PublishedListError, RedisFeedSource,
    connect, parse_spec,
};

/// The preferred default feed: shown when no `?feed=` is requested, if the
/// publisher advertises it.
const PREFERRED_DEFAULT: (Order, Limit) = (Order::Quality, Limit::weeks(4));

/// The published lists the home page can show, discovered from the publisher's
/// feed catalog. Holds one reader per list, all against the single publish redis
/// url.
pub struct AvailableFeeds {
    /// Specs in dropdown order (quality before recency, ascending window);
    /// non-empty by construction.
    specs: Vec<(Order, Limit)>,
    /// The default selection — [`PREFERRED_DEFAULT`] if present, else `specs[0]`.
    default: (Order, Limit),
    readers: HashMap<(Order, Limit), RedisFeedSource>,
}

/// Dropdown sort key: quality feeds before recency, then ascending window. The
/// `i64` window is millis from `chrono::Duration` (always ≥ 0 here).
const fn dropdown_key((order, limit): (Order, Limit)) -> (u8, i64) {
    let order_rank = match order {
        Order::Quality => 0,
        Order::Recency => 1,
    };
    (order_rank, limit.window().num_milliseconds())
}

/// One dropdown option: its `{order}-{limit}` value and whether it is selected.
pub struct FeedOption {
    pub value: String,
    pub selected: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("no feeds in the publisher's catalog (has skeet-publish run yet?)")]
pub struct NoFeedsConfigured;

/// A `?feed=` value that doesn't name one of the configured feeds (unparseable,
/// or a valid spec that wasn't configured).
#[derive(Debug, thiserror::Error)]
#[error("unknown feed: \"{0}\"")]
pub struct UnknownFeed(pub String);

fn spec_value((order, limit): (Order, Limit)) -> String {
    format!("{order}-{limit}")
}

impl AvailableFeeds {
    /// Build a reader per discovered spec against `redis_url`. Specs are sorted
    /// into dropdown order (quality before recency, ascending window) and
    /// deduplicated; the default is [`PREFERRED_DEFAULT`] if present, else the
    /// first in that order. Errors if `specs` is empty.
    pub fn new(
        redis_url: impl Into<Arc<str>>,
        mut specs: Vec<(Order, Limit)>,
    ) -> Result<Self, NoFeedsConfigured> {
        specs.sort_by_key(|&spec| dropdown_key(spec));
        specs.dedup();
        let &first = specs.first().ok_or(NoFeedsConfigured)?;
        let default = if specs.contains(&PREFERRED_DEFAULT) {
            PREFERRED_DEFAULT
        } else {
            first
        };
        let url: Arc<str> = redis_url.into();
        let readers = specs
            .iter()
            .map(|&(order, limit)| {
                (
                    (order, limit),
                    RedisFeedSource::new(url.as_ref(), order, limit),
                )
            })
            .collect();
        Ok(Self {
            specs,
            default,
            readers,
        })
    }

    /// The default spec used when no feed is explicitly requested.
    pub const fn default_spec(&self) -> (Order, Limit) {
        self.default
    }

    /// Resolve a requested `{order}-{limit}` value to a configured feed. An absent
    /// value uses the default; an explicit value that isn't a configured feed is
    /// an error (rather than silently falling back).
    pub fn resolve(&self, requested: Option<&str>) -> Result<(Order, Limit), UnknownFeed> {
        requested.map_or_else(
            || Ok(self.default_spec()),
            |r| {
                parse_spec(r)
                    .ok()
                    .filter(|spec| self.readers.contains_key(spec))
                    .ok_or_else(|| UnknownFeed(r.to_string()))
            },
        )
    }

    /// The reader for a configured spec, if any.
    pub fn reader(&self, spec: (Order, Limit)) -> Option<&RedisFeedSource> {
        self.readers.get(&spec)
    }

    /// The dropdown options in configured order, marking `selected`.
    pub fn options(&self, selected: (Order, Limit)) -> Vec<FeedOption> {
        self.specs
            .iter()
            .map(|&spec| FeedOption {
                value: spec_value(spec),
                selected: spec == selected,
            })
            .collect()
    }
}

/// Discovers the currently-available feeds by reading the publisher's catalog
/// from redis, building a fresh [`AvailableFeeds`] each time.
///
/// Held by the home request path and consulted per render, so feeds published
/// after skeet-appraise started up are picked up without a restart.
pub struct PublishedListCatalogReader {
    redis_url: Arc<str>,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("connecting to publish redis: {0}")]
    Connect(#[from] RedisError),
    #[error("reading feed catalog: {0}")]
    Catalog(#[from] PublishedListError),
    #[error(transparent)]
    NoFeeds(#[from] NoFeedsConfigured),
}

impl PublishedListCatalogReader {
    pub fn new(redis_url: impl Into<Arc<str>>) -> Self {
        Self {
            redis_url: redis_url.into(),
        }
    }

    /// Read the publisher's catalog and build the available feeds from it.
    pub async fn discover(&self) -> Result<AvailableFeeds, DiscoverError> {
        let mut conn = connect(&self.redis_url).await?;
        let specs = PublishedListCatalog::read(&mut conn)
            .await?
            .iter()
            .map(PublishedList::spec)
            .collect();
        Ok(AvailableFeeds::new(Arc::clone(&self.redis_url), specs)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "redis://127.0.0.1:1";

    /// Specs deliberately passed out of dropdown order, to prove `new` sorts them.
    fn feeds() -> AvailableFeeds {
        AvailableFeeds::new(
            URL,
            vec![
                (Order::Recency, Limit::hours(48)),
                (Order::Quality, Limit::days(7)),
                (Order::Quality, Limit::hours(48)),
            ],
        )
        .expect("non-empty")
    }

    #[test]
    fn rejects_empty_specs() {
        assert!(AvailableFeeds::new(URL, vec![]).is_err());
    }

    #[test]
    fn default_falls_back_to_first_when_preferred_absent() {
        // No quality-4w in this set, so the default is the first in dropdown order.
        assert_eq!(feeds().default_spec(), (Order::Quality, Limit::hours(48)));
    }

    #[test]
    fn default_prefers_quality_4w_when_present() {
        let feeds = AvailableFeeds::new(
            URL,
            vec![
                (Order::Quality, Limit::hours(48)),
                (Order::Quality, Limit::weeks(4)),
                (Order::Recency, Limit::weeks(4)),
            ],
        )
        .expect("non-empty");
        assert_eq!(feeds.default_spec(), (Order::Quality, Limit::weeks(4)));
    }

    #[test]
    fn deduplicates_repeated_specs() {
        let feeds = AvailableFeeds::new(
            URL,
            vec![
                (Order::Quality, Limit::hours(48)),
                (Order::Quality, Limit::hours(48)),
            ],
        )
        .expect("non-empty");
        let opts = feeds.options(feeds.default_spec());
        assert_eq!(opts.len(), 1);
    }

    #[test]
    fn resolves_configured_values() {
        let feeds = feeds();
        assert_eq!(
            feeds.resolve(Some("quality-7d")).expect("configured"),
            (Order::Quality, Limit::days(7))
        );
        assert_eq!(
            feeds.resolve(Some("recency-48h")).expect("configured"),
            (Order::Recency, Limit::hours(48))
        );
    }

    #[test]
    fn absent_value_uses_the_default() {
        assert_eq!(
            feeds().resolve(None).expect("default"),
            (Order::Quality, Limit::hours(48))
        );
    }

    #[test]
    fn unknown_or_unconfigured_value_is_an_error() {
        let feeds = feeds();
        assert!(feeds.resolve(Some("nonsense")).is_err());
        // A valid spec that wasn't configured isn't selectable.
        assert!(feeds.resolve(Some("recency-7d")).is_err());
    }

    #[test]
    fn options_are_in_dropdown_order_and_mark_the_selected() {
        let feeds = feeds();
        let opts = feeds.options(feeds.resolve(Some("quality-7d")).expect("configured"));
        let values: Vec<&str> = opts.iter().map(|o| o.value.as_str()).collect();
        // Quality before recency, ascending window — regardless of input order.
        assert_eq!(values, ["quality-48h", "quality-7d", "recency-48h"]);
        let selected: Vec<&str> = opts
            .iter()
            .filter(|o| o.selected)
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(selected, ["quality-7d"]);
    }

    #[test]
    fn reader_present_only_for_configured_specs() {
        let feeds = feeds();
        assert!(feeds.reader((Order::Quality, Limit::days(7))).is_some());
        assert!(feeds.reader((Order::Recency, Limit::days(7))).is_none());
    }
}
