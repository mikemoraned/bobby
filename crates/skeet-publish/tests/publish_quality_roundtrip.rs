//! End-to-end round-trip: the publisher writes a `quality-48h` list and a
//! `RedisFeedSource` on the same `(order, limit)` reads the skeets back in
//! quality order (band, then normalised score, best first).
//!
//! Self-contained (no shared `common` module) like `redis_published_list.rs`.
//! Requires Docker; the `_docker` name is filtered out by `just test-no-docker`.

use std::sync::Arc;
use std::time::Duration;

use bluesky::StaticExistenceChecker;
use chrono::Utc;
use shared::{BlueskyCid, DiscoveredAt, ImageId, OriginalAt, Zone};
use skeet_publish::{
    CdnImageUrlResolver, FeedPublisher, FeedSource, Limit, Order, PublishedList, RedisFeedSource,
    connect,
};
use skeet_store::test_utils::{open_temp_store, test_image};
use skeet_store::{
    ImageRecord, Images, ModelScore, ModelVersion, PruneStats, Score, Scores, SkeetStore,
    Statistics,
};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::time::{Instant, sleep};

// Distinct, valid CIDv1 (raw, sha2-256) strings — only distinct `V3` image ids
// matter, so the CDN resolver succeeds for each.
const CIDS: [&str; 4] = [
    "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiacaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiadaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
];

fn scored_record_at(rkey: &str, cid: &str, original_at: OriginalAt) -> ImageRecord {
    ImageRecord {
        image_id: ImageId::V3(BlueskyCid::new(cid).expect("valid cid")),
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse()
            .expect("valid skeet id"),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at,
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    }
}

fn scored_record(rkey: &str, cid: &str) -> ImageRecord {
    scored_record_at(rkey, cid, OriginalAt::new(Utc::now()))
}

/// Seed one scored image into `store` at the given age and raw score.
async fn seed_scored(store: &SkeetStore, record: ImageRecord, score: f32) {
    let mv = ModelVersion::from("test");
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &record.image_id,
            ModelScore {
                score: Score::new(score).expect("valid score"),
                model_version: mv.clone(),
            },
        )
        .await
        .expect("upsert score");
}

/// The rkeys a `RedisFeedSource` reads back, in list order.
async fn rkeys(reader: &RedisFeedSource) -> Vec<String> {
    let skeleton = reader.skeleton(false).await.expect("read skeleton");
    skeleton
        .skeet_ids
        .iter()
        .map(|s| s.rkey().as_str().to_string())
        .collect()
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
            .upsert_score(
                &record.image_id,
                ModelScore {
                    score: Score::new(score).expect("valid score"),
                    model_version: mv.clone(),
                },
            )
            .await
            .expect("upsert score");
    }
}

async fn start_redis() -> (ContainerAsync<Redis>, String) {
    let container = Redis::default()
        .start()
        .await
        .expect("start redis container");
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
        Arc::new(StaticExistenceChecker::all_present()),
        vec![(Order::Quality, Limit::hours(48))],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher
        .publish(&mut conn, Utc::now())
        .await
        .expect("publish");

    // A reader on the same (order, limit) gets the skeets back in quality order:
    // High first (b_high), then MedHigh by score (a_med 0.60 before c_med 0.55).
    let reader = RedisFeedSource::new(url, Order::Quality, Limit::hours(48));
    assert_eq!(rkeys(&reader).await, ["b_high", "a_med", "c_med"]);
}


/// The publisher writes per-list statistics during a publish cycle: the absolute
/// window it covers, the images examined over that window (summed from recorded
/// prune stats), and how many it ended up showing (the list length).
#[tokio::test]
async fn list_statistics_published_and_read_back_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed_quality(&store).await; // three visible scored images, all published "now"

    // One recorded interval inside the 48h window contributes the examined count.
    let now = Utc::now();
    store
        .record_prune_stats(&PruneStats {
            interval_start: now - chrono::Duration::hours(1),
            interval_end: now,
            skeets_seen: 5000,
            images_examined: 5000,
            images_saved: 3,
        })
        .await
        .expect("record stats");

    let (_container, url) = start_redis().await;

    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        test_support::test_models(),
        Arc::new(CdnImageUrlResolver),
        Arc::new(StaticExistenceChecker::all_present()),
        vec![(Order::Quality, Limit::hours(48))],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher.publish(&mut conn, now).await.expect("publish");

    let stats = PublishedList::new(Order::Quality, Limit::hours(48))
        .read_statistics(&mut conn)
        .await
        .expect("read statistics")
        .expect("statistics present");
    // Window is the list's absolute span, ending at the publish time.
    assert_eq!(stats.interval_start, now - chrono::Duration::hours(48));
    assert_eq!(stats.interval_end, now);
    assert_eq!(stats.examined, 5000);
    assert_eq!(stats.found, 3); // all three seeded skeets are visible within 48h
    // The existence checker reports all present, so every candidate is live.
    assert_eq!(stats.exists, 3);
}

/// Publisher-side consistency invariant: the `exists` it records equals the number
/// of live items in the list it just wrote. With one candidate's skeet probed
/// missing, `found` counts all three but `exists` counts only the two live ones —
/// and that matches what re-reading the list and applying `is_live` yields.
#[tokio::test]
async fn published_exists_matches_live_items_in_the_written_list_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed_quality(&store).await; // a_med, b_high, c_med — all visible within 48h

    let (_container, url) = start_redis().await;

    // Probe c_med's skeet as deleted, so it stays in the list but is not live.
    let missing_skeet = "at://did:plc:abc/app.bsky.feed.post/c_med"
        .parse()
        .expect("valid skeet id");
    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        test_support::test_models(),
        Arc::new(CdnImageUrlResolver),
        Arc::new(StaticExistenceChecker::all_present().with_missing_skeets([missing_skeet])),
        vec![(Order::Quality, Limit::hours(48))],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher.publish(&mut conn, Utc::now()).await.expect("publish");

    let list = PublishedList::new(Order::Quality, Limit::hours(48));
    let written = list.read(&mut conn).await.expect("read list");
    let stats = list
        .read_statistics(&mut conn)
        .await
        .expect("read statistics")
        .expect("statistics present");

    let live_in_list = written.iter().filter(|p| p.is_live()).count() as u64;
    assert_eq!(stats.found, written.len() as u64); // all candidates kept in the list
    assert_eq!(stats.exists, live_in_list); // exists tracks exactly the live ones
    assert_eq!(stats.exists, 2); // c_med dropped, a_med + b_high remain
}

/// The wider `quality-7d` window includes a High-band skeet published 4 days ago
/// that the `quality-48h` window excludes — same publisher, two specs differing
/// only in window. Order stays quality (band, then score) in both lists.
#[tokio::test]
async fn quality_7d_window_includes_older_skeets_excluded_from_48h_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed_quality(&store).await; // b_high(now, 0.90), a_med(now, 0.60), c_med(now, 0.55)

    // A High-band skeet published 4 days ago: inside 7d, outside 48h.
    let four_days_ago = OriginalAt::new(Utc::now() - chrono::Duration::days(4));
    seed_scored(
        &store,
        scored_record_at("d_old", CIDS[3], four_days_ago),
        0.80,
    )
    .await;

    let (_container, url) = start_redis().await;

    // One publisher, both specs — the generic (order, limit) loop writes each list.
    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        test_support::test_models(),
        Arc::new(CdnImageUrlResolver),
        Arc::new(StaticExistenceChecker::all_present()),
        vec![
            (Order::Quality, Limit::hours(48)),
            (Order::Quality, Limit::days(7)),
        ],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher
        .publish(&mut conn, Utc::now())
        .await
        .expect("publish");

    // 48h excludes the 4-day-old skeet.
    let reader_48h = RedisFeedSource::new(url.clone(), Order::Quality, Limit::hours(48));
    assert_eq!(rkeys(&reader_48h).await, ["b_high", "a_med", "c_med"]);

    // 7d includes it, still in quality order: High by score (b_high 0.90, d_old
    // 0.80), then MedHigh by score (a_med 0.60, c_med 0.55).
    let reader_7d = RedisFeedSource::new(url, Order::Quality, Limit::days(7));
    assert_eq!(
        rkeys(&reader_7d).await,
        ["b_high", "d_old", "a_med", "c_med"]
    );
}
