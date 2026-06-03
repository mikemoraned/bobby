//! Equivalence test: the redis publish path ≡ the library path.
//!
//! Seeds a store, runs `FeedPublisher` to write the `recency-48h` list to a
//! testcontainers redis, reads it back through `RedisFeedSource`, and asserts
//! the visible skeet-id **set** matches what `LiveFeedSource` produces from the
//! same store. We compare *sets*, not order: the publisher orders by recency
//! while the library orders by score (see the ordering decision in
//! `docs/current-slice.md`). A pair round-trip also confirms the reader's
//! skeet-ids are exactly the order-preserving dedup of the stored pairs.
//!
//! Requires Docker; the `_docker` test is filtered out by `just test-no-docker`.
//! The non-docker `hardcoded_cids_are_valid_and_distinct` validates the fixtures
//! without Docker.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use common::{CIDS, redis_url, seed, v3, wait_ready};
use skeet_publish::{
    CdnImageUrlResolver, FeedCache, FeedPublisher, FeedSource, Limit, LiveFeedSource, Order,
    PublishedList, RedisFeedSource, connect,
};
use skeet_store::SkeetId;
use skeet_store::test_utils::open_temp_store;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::Redis;

fn id_set(skeet_ids: &[SkeetId]) -> HashSet<String> {
    skeet_ids.iter().map(ToString::to_string).collect()
}

#[test]
fn hardcoded_cids_are_valid_and_distinct() {
    let ids: HashSet<String> = (0..CIDS.len()).map(|i| v3(i).to_string()).collect();
    assert_eq!(ids.len(), CIDS.len(), "cids must parse and be distinct");
}

#[tokio::test]
async fn redis_path_matches_library_path_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed(&store).await;
    let models = test_support::test_models();
    let spec = (Order::Recency, Limit::hours(48));

    // Library path: compute the skeleton in-process.
    let cache = Arc::new(FeedCache::new(Arc::clone(&store), Arc::clone(&models), 50, 48));
    let library = LiveFeedSource::new(cache);
    let library_skeleton = library.skeleton(false).await.expect("library skeleton");
    let library_set = id_set(&library_skeleton.skeet_ids);
    assert_eq!(library_set.len(), 3, "three visible skeets via the library");

    // Redis path: publish, then read back.
    let container = Redis::default().start().await.expect("start redis");
    let url = redis_url(&container).await;
    wait_ready(&url).await;

    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        Arc::clone(&models),
        Arc::new(CdnImageUrlResolver),
        vec![spec],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher
        .publish(&mut conn, Utc::now())
        .await
        .expect("publish");

    let redis_source = RedisFeedSource::new(url.clone(), spec.0, spec.1);
    let redis_skeleton = redis_source.skeleton(false).await.expect("redis skeleton");
    let redis_set = id_set(&redis_skeleton.skeet_ids);

    // The core guarantee: both paths admit exactly the same visible skeets.
    assert_eq!(redis_set, library_set);

    // Pair round-trip: one pair per visible skeet, and the reader's skeet-ids
    // are exactly their order-preserving dedup.
    let pairs = PublishedList::new(spec.0, spec.1)
        .read(&mut conn)
        .await
        .expect("read pairs");
    assert_eq!(pairs.len(), 3);
    let from_pairs: Vec<String> = pairs.iter().map(|p| p.skeet_id.to_string()).collect();
    let from_reader: Vec<String> = redis_skeleton
        .skeet_ids
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(from_reader, from_pairs);
}
