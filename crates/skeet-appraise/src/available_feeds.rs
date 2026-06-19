use std::collections::HashMap;
use std::sync::Arc;

use skeet_publish::{Limit, Order, RedisFeedSource, parse_spec};

/// The published lists the home page can show, configured and ordered from the
/// `--publish` args (the first is the default selection). Holds one reader per
/// list, all against the single publish redis url.
pub struct AvailableFeeds {
    /// Configured specs in dropdown order; non-empty by construction, with
    /// `specs[0]` the default.
    specs: Vec<(Order, Limit)>,
    readers: HashMap<(Order, Limit), RedisFeedSource>,
}

/// One dropdown option: its `{order}-{limit}` value and whether it is selected.
pub struct FeedOption {
    pub value: String,
    pub selected: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("no feeds configured (need at least one --publish spec)")]
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
    /// Build a reader per spec against `redis_url`, preserving `specs` order. The
    /// first spec is the default. Errors if `specs` is empty.
    pub fn new(
        redis_url: impl Into<Arc<str>>,
        specs: Vec<(Order, Limit)>,
    ) -> Result<Self, NoFeedsConfigured> {
        if specs.is_empty() {
            return Err(NoFeedsConfigured);
        }
        let url: Arc<str> = redis_url.into();
        let readers = specs
            .iter()
            .map(|&(order, limit)| ((order, limit), RedisFeedSource::new(url.as_ref(), order, limit)))
            .collect();
        Ok(Self { specs, readers })
    }

    /// The default (first-configured) spec.
    pub fn default_spec(&self) -> (Order, Limit) {
        self.specs[0]
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

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "redis://127.0.0.1:1";

    fn feeds() -> AvailableFeeds {
        AvailableFeeds::new(
            URL,
            vec![
                (Order::Quality, Limit::hours(48)),
                (Order::Quality, Limit::days(7)),
                (Order::Recency, Limit::hours(48)),
            ],
        )
        .expect("non-empty")
    }

    #[test]
    fn rejects_empty_specs() {
        assert!(AvailableFeeds::new(URL, vec![]).is_err());
    }

    #[test]
    fn default_is_first_configured() {
        assert_eq!(feeds().default_spec(), (Order::Quality, Limit::hours(48)));
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
        assert_eq!(feeds().resolve(None).expect("default"), (Order::Quality, Limit::hours(48)));
    }

    #[test]
    fn unknown_or_unconfigured_value_is_an_error() {
        let feeds = feeds();
        assert!(feeds.resolve(Some("nonsense")).is_err());
        // A valid spec that wasn't configured isn't selectable.
        assert!(feeds.resolve(Some("recency-7d")).is_err());
    }

    #[test]
    fn options_are_in_order_and_mark_the_selected() {
        let feeds = feeds();
        let opts = feeds.options(feeds.resolve(Some("quality-7d")).expect("configured"));
        let values: Vec<&str> = opts.iter().map(|o| o.value.as_str()).collect();
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
