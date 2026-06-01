//! Equivalence test: the redis publish path ≡ the library path.
//!
//! Seeds a store, runs `FeedPublisher` to write the `recency-48h` list to a
//! testcontainers redis, reads it back through `RedisFeedSource`, and asserts
//! the visible skeet-id **set** matches what `LiveFeedSource` produces from the
//! same store. We compare *sets*, not order: the publisher orders by recency
//! while the library orders by score (see the ordering decision in
//! `docs/current-slice.md`). A pair round-trip also confirms the reader's
//! skeet-ids are exactly the order-preserving dedup of the stored pairs.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use shared::{BlueskyCid, ImageId};
use skeet_publish::{
    CdnImageUrlResolver, FeedCache, FeedPublisher, FeedSource, Limit, LiveFeedSource, Order,
    PublishedList, RedisFeedSource, connect,
};
use skeet_store::test_utils::{open_temp_store, test_image};
use skeet_store::{DiscoveredAt, ImageRecord, ModelVersion, OriginalAt, Score, SkeetStore, Zone};
use test_support::test_models;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::time::{Instant, sleep};

/// Distinct, valid CIDv1 (raw, sha2-256) strings — content is irrelevant; we
/// only need distinct `V3` image ids so the CDN resolver succeeds for each.
const CIDS: [&str; 4] = [
    "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiacaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiadaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
];

fn v3(i: usize) -> ImageId {
    ImageId::V3(BlueskyCid::new(CIDS[i]).expect("valid cid"))
}

fn scored_record(rkey: &str, image_id: ImageId) -> ImageRecord {
    ImageRecord {
        image_id,
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse()
            .expect("valid skeet id"),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    }
}

/// Seed three visible skeets (score ≥ 0.5 for the `test` model) and one hidden
/// (score < 0.5), all `V3` and published now.
async fn seed(store: &SkeetStore) {
    let mv = ModelVersion::from("test");
    let rows = [
        ("vis1", 0, 0.9),
        ("vis2", 1, 0.6),
        ("vis3", 2, 0.55),
        ("hidden", 3, 0.1),
    ];
    for (rkey, cid_idx, score) in rows {
        let record = scored_record(rkey, v3(cid_idx));
        store.add(&record).await.expect("add record");
        store
            .upsert_score(
                &record.image_id,
                &Score::new(score).expect("valid score"),
                &mv,
            )
            .await
            .expect("upsert score");
    }
}

#[test]
fn hardcoded_cids_are_valid_and_distinct() {
    let ids: HashSet<String> = (0..CIDS.len()).map(|i| v3(i).to_string()).collect();
    assert_eq!(ids.len(), CIDS.len(), "cids must parse and be distinct");
}

async fn redis_url(container: &ContainerAsync<Redis>) -> String {
    let host = container.get_host().await.expect("host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("mapped port");
    format!("redis://{host}:{port}")
}

/// Wait until the container actually answers before using it (Docker's host
/// port-forward can refuse briefly after the container reports ready).
async fn wait_ready(url: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if connect(url).await.is_ok() {
            return;
        }
        assert!(Instant::now() < deadline, "redis not ready within 30s");
        sleep(Duration::from_millis(100)).await;
    }
}

fn id_set(skeet_ids: &[skeet_store::SkeetId]) -> HashSet<String> {
    skeet_ids.iter().map(ToString::to_string).collect()
}

#[tokio::test]
async fn redis_path_matches_library_path_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed(&store).await;
    let models = test_models();
    let spec = (Order::Recency, Limit::hours(48));

    // Library path: compute the skeleton in-process.
    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        Arc::clone(&models),
        50,
        48,
    ));
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
