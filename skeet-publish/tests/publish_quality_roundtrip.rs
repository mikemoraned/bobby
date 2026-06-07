//! End-to-end round-trip: the publisher writes a `quality-48h` list and a
//! `RedisFeedSource` on the same `(order, limit)` reads the skeets back in
//! quality order (band, then normalised score, best first).
//!
//! Self-contained (no shared `common` module) like `redis_published_list.rs`.
//! Requires Docker; the `_docker` name is filtered out by `just test-no-docker`.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use shared::{BlueskyCid, ImageId};
use skeet_publish::{
    CdnImageUrlResolver, FeedPublisher, FeedSource, Limit, Order, RedisFeedSource, connect,
};
use skeet_store::test_utils::{open_temp_store, test_image};
use skeet_store::{
    DiscoveredAt, ImageRecord, ModelVersion, OriginalAt, Score, SkeetStore, Zone,
};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::time::{Instant, sleep};

// Distinct, valid CIDv1 (raw, sha2-256) strings — only distinct `V3` image ids
// matter, so the CDN resolver succeeds for each.
const CIDS: [&str; 3] = [
    "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiacaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
];

fn scored_record(rkey: &str, cid: &str) -> ImageRecord {
    ImageRecord {
        image_id: ImageId::V3(BlueskyCid::new(cid).expect("valid cid")),
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

/// Seed three visible skeets whose *insertion* order (a, b, c) deliberately
/// differs from their *quality* order, so a no-op sort would fail the assertion.
/// With the `test` model (threshold 0.5) the normalised score equals the raw
/// score: `b` bands High (0.90); `a` and `c` band MediumHigh (0.60, 0.55).
async fn seed_quality(store: &SkeetStore) {
    let mv = ModelVersion::from("test");
    let rows = [("a_med", 0, 0.60), ("b_high", 1, 0.90), ("c_med", 2, 0.55)];
    for (rkey, cid_idx, score) in rows {
        let record = scored_record(rkey, CIDS[cid_idx]);
        store.add(&record).await.expect("add record");
        store
            .upsert_score(&record.image_id, &Score::new(score).expect("valid score"), &mv)
            .await
            .expect("upsert score");
    }
}

async fn start_redis() -> (ContainerAsync<Redis>, String) {
    let container = Redis::default().start().await.expect("start redis container");
    let host = container.get_host().await.expect("host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("mapped port");
    let url = format!("redis://{host}:{port}");

    // Docker's host port-forward can refuse briefly after the container is ready.
    let deadline = Instant::now() + Duration::from_secs(30);
    while connect(&url).await.is_err() {
        assert!(Instant::now() < deadline, "redis not ready within 30s");
        sleep(Duration::from_millis(100)).await;
    }
    (container, url)
}

#[tokio::test]
async fn quality_list_roundtrips_publisher_to_reader_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed_quality(&store).await;

    let (_container, url) = start_redis().await;

    // Publisher writes the quality-48h list from the seeded store.
    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        test_support::test_models(),
        Arc::new(CdnImageUrlResolver),
        vec![(Order::Quality, Limit::hours(48))],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher.publish(&mut conn, Utc::now()).await.expect("publish");

    // A reader on the same (order, limit) gets the skeets back in quality order:
    // High first (b_high), then MedHigh by score (a_med 0.60 before c_med 0.55).
    let reader = RedisFeedSource::new(url, Order::Quality, Limit::hours(48));
    let skeleton = reader.skeleton(false).await.expect("read skeleton");
    let rkeys: Vec<String> = skeleton
        .skeet_ids
        .iter()
        .map(|s| s.rkey().as_str().to_string())
        .collect();
    assert_eq!(rkeys, ["b_high", "a_med", "c_med"]);
}
