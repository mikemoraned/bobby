use async_trait::async_trait;

use crate::limit::Limit;
use crate::order::Order;
use crate::published_list::{PublishedList, PublishedListError};
use crate::published_list_catalog::PublishedListCatalog;
use crate::redis_client::connect;
use crate::source::{
    FeedSkeleton, FeedSource, FeedSourceError, PublishedImages, PublishedImagesSource,
    RedisFeedSource,
};

/// Build the fallback chain for `preferred` out of the `available` specs.
///
/// Keeps every available spec with the same [`Order`] as `preferred` and a window
/// (see [`Limit::window`]) at least as wide as the preferred's, sorted ascending
/// by window. The result is the order in which lists should be tried: the
/// preferred list first if it is available, otherwise the next-oldest, widening
/// to the oldest. It never includes a window narrower (newer) than the preferred,
/// never crosses `Order`, and is empty only when no same-order list is wide
/// enough.
pub fn fallback_chain(
    available: &[(Order, Limit)],
    preferred: (Order, Limit),
) -> Vec<(Order, Limit)> {
    let (preferred_order, preferred_limit) = preferred;
    let mut chain: Vec<(Order, Limit)> = available
        .iter()
        .copied()
        .filter(|(order, limit)| {
            *order == preferred_order && limit.window() >= preferred_limit.window()
        })
        .collect();
    chain.sort_by_key(|(_, limit)| limit.window());
    chain.dedup();
    chain
}

/// Reads the catalog and individual lists for [`FallbackFeedSource`]. Abstracted
/// behind a trait so the first-non-empty selection can be tested without redis.
#[async_trait]
trait FallbackReader: Send + Sync {
    /// The specs currently advertised in the publisher's catalog.
    async fn available_specs(&self) -> Result<Vec<(Order, Limit)>, FeedSourceError>;
    async fn skeleton(&self, spec: (Order, Limit)) -> Result<FeedSkeleton, FeedSourceError>;
    async fn published_images(
        &self,
        spec: (Order, Limit),
    ) -> Result<PublishedImages, FeedSourceError>;
    async fn examined_count(&self) -> Result<Option<u64>, FeedSourceError>;
}

/// Redis-backed [`FallbackReader`]: reads the catalog over a fresh connection and
/// reuses [`RedisFeedSource`] (with its transient-retry) to read each list.
struct RedisFallbackReader {
    url: String,
    /// A reader used only for [`PublishedImagesSource::examined_count`], which
    /// reads a single global key independent of any list — so the spec it carries
    /// is irrelevant to that read.
    examined_count_reader: RedisFeedSource,
}

impl RedisFallbackReader {
    fn new(url: impl Into<String>, preferred: (Order, Limit)) -> Self {
        let url = url.into();
        let examined_count_reader = RedisFeedSource::new(url.clone(), preferred.0, preferred.1);
        Self {
            url,
            examined_count_reader,
        }
    }
}

#[async_trait]
impl FallbackReader for RedisFallbackReader {
    async fn available_specs(&self) -> Result<Vec<(Order, Limit)>, FeedSourceError> {
        let mut conn = connect(&self.url).await.map_err(PublishedListError::from)?;
        let specs = PublishedListCatalog::read(&mut conn)
            .await?
            .iter()
            .map(PublishedList::spec)
            .collect();
        Ok(specs)
    }

    async fn skeleton(&self, spec: (Order, Limit)) -> Result<FeedSkeleton, FeedSourceError> {
        RedisFeedSource::new(&self.url, spec.0, spec.1)
            .skeleton(false)
            .await
    }

    async fn published_images(
        &self,
        spec: (Order, Limit),
    ) -> Result<PublishedImages, FeedSourceError> {
        RedisFeedSource::new(&self.url, spec.0, spec.1)
            .published_images()
            .await
    }

    async fn examined_count(&self) -> Result<Option<u64>, FeedSourceError> {
        self.examined_count_reader.examined_count().await
    }
}

/// A [`FeedSource`]/[`PublishedImagesSource`] that degrades to older data when the
/// preferred list is empty or missing.
///
/// Per call it re-reads the publisher's catalog, builds the [`fallback_chain`] for
/// the preferred `(Order, Limit)`, and returns the first non-empty list in that
/// chain — so an outage that empties the preferred list (or a newly-published
/// older one) is picked up live, without a restart. If every list in the chain is
/// empty or missing, the last (empty) result is returned so the surface still
/// renders. The happy path is one catalog read plus one list read; only when the
/// preferred list is empty does it read further.
pub struct FallbackFeedSource {
    reader: Box<dyn FallbackReader>,
    preferred: (Order, Limit),
}

impl FallbackFeedSource {
    pub fn new(url: impl Into<String>, order: Order, limit: Limit) -> Self {
        let url = url.into();
        Self {
            reader: Box::new(RedisFallbackReader::new(url, (order, limit))),
            preferred: (order, limit),
        }
    }

    /// The fallback chain for this call: re-read the catalog and order the
    /// same-order, wide-enough lists oldest-last (see [`fallback_chain`]).
    async fn chain(&self) -> Result<Vec<(Order, Limit)>, FeedSourceError> {
        let specs = self.reader.available_specs().await?;
        Ok(fallback_chain(&specs, self.preferred))
    }
}

#[async_trait]
impl FeedSource for FallbackFeedSource {
    async fn skeleton(&self, _force_refresh: bool) -> Result<FeedSkeleton, FeedSourceError> {
        let mut last = None;
        for spec in self.chain().await? {
            let result = self.reader.skeleton(spec).await?;
            if result.skeet_ids.is_empty() {
                last = Some(result);
            } else {
                return Ok(result);
            }
        }
        Ok(last.unwrap_or(FeedSkeleton {
            skeet_ids: vec![],
            refreshed_at: None,
        }))
    }
}

#[async_trait]
impl PublishedImagesSource for FallbackFeedSource {
    async fn published_images(&self) -> Result<PublishedImages, FeedSourceError> {
        let mut last = None;
        for spec in self.chain().await? {
            let result = self.reader.published_images(spec).await?;
            if result.images.is_empty() {
                last = Some(result);
            } else {
                return Ok(result);
            }
        }
        Ok(last.unwrap_or(PublishedImages {
            images: vec![],
            refreshed_at: None,
        }))
    }

    async fn examined_count(&self) -> Result<Option<u64>, FeedSourceError> {
        self.reader.examined_count().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use chrono::{DateTime, TimeZone, Utc};
    use proptest::prelude::*;
    use shared::SkeetId;

    fn arbitrary_spec() -> impl Strategy<Value = (Order, Limit)> {
        let order = prop_oneof![Just(Order::Quality), Just(Order::Recency)];
        let limit = (1u64..1000, 0usize..4).prop_map(|(count, unit)| match unit {
            0 => Limit::hours(count),
            1 => Limit::days(count),
            2 => Limit::weeks(count),
            _ => Limit::years(count),
        });
        (order, limit)
    }

    proptest! {
        /// For any set of available specs and any preferred spec, every entry in
        /// the resulting fallback chain must satisfy the four invariants that make
        /// it a valid "degrade to older data" sequence — checked here against
        /// random inputs rather than hand-picked examples:
        ///
        /// 1. **Same Order.** Every entry in the chain has the same `Order`.
        /// 2. **Every entry in the chain is older than preferred.** Every entry's
        ///    window is ≥ the preferred's, so degradation only ever moves to
        ///    *wider/older* data, never to a fresher window than was asked for.
        /// 3. **Non-decreasing window.** Entries widen monotonically, so the
        ///    chain is tried oldest-last (preferred-or-next-oldest first).
        /// 4. **Drawn from the available specs.** Every entry was actually present
        ///    in `available` — the chain never invents a list that wasn't there.
        #[test]
        fn chain_is_same_order_nondecreasing_and_never_newer(
            available in prop::collection::vec(arbitrary_spec(), 0..10),
            preferred in arbitrary_spec(),
        ) {
            let chain = fallback_chain(&available, preferred);
            let preferred_order = preferred.0;
            let preferred_window = preferred.1.window();

            // (1) same Order
            prop_assert!(chain.iter().all(|&(order, _)| order == preferred_order));
            // (2) all older than preferred
            prop_assert!(chain.iter().all(|&(_, limit)| limit.window() >= preferred_window));
            // (3) non-decreasing window (oldest-last)
            prop_assert!(chain.is_sorted_by_key(|&(_, limit)| limit.window()));
            // (4) every entry was in the available specs
            prop_assert!(chain.iter().all(|spec| available.contains(spec)));
        }
    }

    #[test]
    fn chain_starts_at_preferred_and_widens() {
        let available = vec![
            (Order::Recency, Limit::hours(48)),
            (Order::Quality, Limit::years(1)),
            (Order::Quality, Limit::hours(48)),
            (Order::Quality, Limit::weeks(4)),
            (Order::Quality, Limit::days(7)),
        ];
        assert_eq!(
            fallback_chain(&available, (Order::Quality, Limit::hours(48))),
            vec![
                (Order::Quality, Limit::hours(48)),
                (Order::Quality, Limit::days(7)),
                (Order::Quality, Limit::weeks(4)),
                (Order::Quality, Limit::years(1)),
            ]
        );
    }

    #[test]
    fn chain_starts_at_next_oldest_when_preferred_missing() {
        let available = vec![
            (Order::Quality, Limit::days(7)),
            (Order::Quality, Limit::weeks(4)),
        ];
        assert_eq!(
            fallback_chain(&available, (Order::Quality, Limit::hours(48))),
            vec![
                (Order::Quality, Limit::days(7)),
                (Order::Quality, Limit::weeks(4)),
            ]
        );
    }

    #[test]
    fn chain_excludes_newer_than_preferred_and_other_orders() {
        let available = vec![
            (Order::Quality, Limit::hours(48)),
            (Order::Quality, Limit::weeks(4)),
            (Order::Recency, Limit::years(1)),
        ];
        // Preferred 4w: 48h is newer (excluded), recency-1y is a different order.
        assert_eq!(
            fallback_chain(&available, (Order::Quality, Limit::weeks(4))),
            vec![(Order::Quality, Limit::weeks(4))]
        );
    }

    #[test]
    fn chain_empty_when_no_same_order_is_wide_enough() {
        let available = vec![(Order::Quality, Limit::hours(48))];
        assert!(fallback_chain(&available, (Order::Quality, Limit::weeks(4))).is_empty());
    }

    /// A fake reader where `populated` specs return one item carrying a
    /// per-spec `refreshed_at` (so a test can tell which list won), and all
    /// others return empty.
    struct FakeReader {
        specs: Vec<(Order, Limit)>,
        populated: HashSet<(Order, Limit)>,
    }

    /// A distinct timestamp per spec, derived from its window, so the winning
    /// list is identifiable from the result's `refreshed_at`.
    fn marker(spec: (Order, Limit)) -> DateTime<Utc> {
        Utc.timestamp_opt(spec.1.window().num_hours(), 0)
            .single()
            .expect("valid timestamp")
    }

    #[async_trait]
    impl FallbackReader for FakeReader {
        async fn available_specs(&self) -> Result<Vec<(Order, Limit)>, FeedSourceError> {
            Ok(self.specs.clone())
        }

        async fn skeleton(&self, spec: (Order, Limit)) -> Result<FeedSkeleton, FeedSourceError> {
            let skeet_ids = if self.populated.contains(&spec) {
                vec![SkeetId::for_post("did:example:test", "rkey")]
            } else {
                vec![]
            };
            Ok(FeedSkeleton {
                skeet_ids,
                refreshed_at: Some(marker(spec)),
            })
        }

        async fn published_images(
            &self,
            _spec: (Order, Limit),
        ) -> Result<PublishedImages, FeedSourceError> {
            unimplemented!("not exercised by these tests")
        }

        async fn examined_count(&self) -> Result<Option<u64>, FeedSourceError> {
            Ok(Some(7))
        }
    }

    fn source_with(
        preferred: (Order, Limit),
        specs: Vec<(Order, Limit)>,
        populated: &[(Order, Limit)],
    ) -> FallbackFeedSource {
        FallbackFeedSource {
            reader: Box::new(FakeReader {
                specs,
                populated: populated.iter().copied().collect(),
            }),
            preferred,
        }
    }

    const PREFERRED: (Order, Limit) = (Order::Quality, Limit::hours(48));
    const NEXT: (Order, Limit) = (Order::Quality, Limit::days(7));
    const WIDEST: (Order, Limit) = (Order::Quality, Limit::weeks(4));

    #[tokio::test]
    async fn populated_preferred_wins() {
        let source = source_with(PREFERRED, vec![PREFERRED, NEXT, WIDEST], &[PREFERRED, NEXT]);
        let skeleton = source.skeleton(false).await.expect("skeleton");
        assert_eq!(skeleton.refreshed_at, Some(marker(PREFERRED)));
    }

    #[tokio::test]
    async fn empty_preferred_falls_back_to_next_non_empty() {
        let source = source_with(PREFERRED, vec![PREFERRED, NEXT, WIDEST], &[WIDEST]);
        let skeleton = source.skeleton(false).await.expect("skeleton");
        assert_eq!(skeleton.refreshed_at, Some(marker(WIDEST)));
    }

    #[tokio::test]
    async fn missing_preferred_starts_at_next_oldest() {
        // Preferred not advertised at all; the chain begins at the next-oldest.
        let source = source_with(PREFERRED, vec![NEXT, WIDEST], &[NEXT]);
        let skeleton = source.skeleton(false).await.expect("skeleton");
        assert_eq!(skeleton.refreshed_at, Some(marker(NEXT)));
    }

    #[tokio::test]
    async fn all_empty_returns_renderable_empty() {
        let source = source_with(PREFERRED, vec![PREFERRED, NEXT, WIDEST], &[]);
        let skeleton = source.skeleton(false).await.expect("skeleton");
        assert!(skeleton.skeet_ids.is_empty());
        // The last candidate's result is carried so the surface still renders.
        assert_eq!(skeleton.refreshed_at, Some(marker(WIDEST)));
    }

    #[tokio::test]
    async fn empty_chain_returns_renderable_empty() {
        // Nothing in the catalog matches the preferred order/window.
        let source = source_with(PREFERRED, vec![(Order::Recency, Limit::years(1))], &[]);
        let skeleton = source.skeleton(false).await.expect("skeleton");
        assert!(skeleton.skeet_ids.is_empty());
        assert_eq!(skeleton.refreshed_at, None);
    }
}
