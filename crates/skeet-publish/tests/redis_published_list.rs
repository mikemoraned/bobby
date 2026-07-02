//! Integration tests for the published-list redis schema against a real redis.
//!
//! A testcontainers redis is started and the `PublishedList` read/write helpers
//! are exercised end-to-end — round-trip, atomic replacement, and empty-clears.
//! This is the in-memory safety net to run before publishing to real Upstash.
//!
//! Requires Docker; the `_docker`-suffixed names are filtered out by the
//! `no-docker` nextest profile (`just test-no-docker`).

use std::time::Duration;

use bluesky::ImageUrl;
use chrono::Utc;
use deadpool_redis::redis::{self, AsyncCommands};
use shared::SkeetId;
use shared::{BlueskyCid, ImageId};
use skeet_publish::{
    FeedSource, Limit, ListStatistics, Order, PublishedImage, PublishedImagesSource, PublishedList,
    PublishedListCatalog, RedisFeedSource,
};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::time::{Instant, sleep};

async fn start_redis() -> (ContainerAsync<Redis>, redis::aio::MultiplexedConnection) {
    let container = Redis::default()
        .start()
        .await
        .expect("start redis container");
    let host = container.get_host().await.expect("get host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("get mapped port");
    let conn = connect_with_retry(&format!("redis://{host}:{port}")).await;
    (container, conn)
}

/// Connect to redis, retrying until it answers a `PING`.
///
/// `Redis::default()` only blocks `start()` until the "Ready to accept
/// connections" log line, but Docker's host port-forward proxy can briefly
/// still refuse connections after that (especially on macOS), so a single
/// connect attempt is racy. Retry until a real round-trip succeeds.
async fn connect_with_retry(url: &str) -> redis::aio::MultiplexedConnection {
    let client = redis::Client::open(url).expect("open redis client");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(mut conn) = client.get_multiplexed_async_connection().await
            && redis::cmd("PING")
                .query_async::<String>(&mut conn)
                .await
                .is_ok()
        {
            return conn;
        }
        assert!(
            Instant::now() < deadline,
            "redis did not accept connections within 30s"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

// Distinct, valid CIDv1 strings — content is irrelevant; we only need distinct,
// parseable `V3` image ids.
const CID_1: &str = "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CID_2: &str = "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CID_3: &str = "bafkreiacaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CID_4: &str = "bafkreiadaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn pair(rkey: &str, cid: &str) -> PublishedImage {
    PublishedImage::unprobed(
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{cid}@jpeg"
        ))
        .expect("valid url"),
        ImageId::V3(BlueskyCid::new(cid).expect("valid cid")),
        format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse::<SkeetId>()
            .expect("valid skeet id"),
    )
}

#[tokio::test]
async fn write_then_read_roundtrips_in_order_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::hours(48));

    let pairs = vec![pair("rk1", CID_1), pair("rk2", CID_2), pair("rk3", CID_3)];
    list.replace(&mut conn, &pairs, Utc::now())
        .await
        .expect("replace");

    let read = list.read(&mut conn).await.expect("read");
    assert_eq!(read, pairs, "read back the same pairs in the same order");

    // It really lives under the {order}-{limit} key.
    let exists: bool = conn.exists("v3-recency-48h").await.expect("exists");
    assert!(exists);
}

#[tokio::test]
async fn replace_swaps_atomically_leaving_no_remnants_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::hours(48));

    let first = vec![pair("rk1", CID_1), pair("rk2", CID_2)];
    list.replace(&mut conn, &first, Utc::now())
        .await
        .expect("first replace");

    // A shorter second list must fully overwrite the first — no stale tail.
    let second = vec![pair("rk9", CID_4)];
    list.replace(&mut conn, &second, Utc::now())
        .await
        .expect("second replace");

    let read = list.read(&mut conn).await.expect("read");
    assert_eq!(read, second);

    // The scratch key used during the swap is gone.
    let leftover: bool = conn
        .exists("v3-recency-48h:building")
        .await
        .expect("exists");
    assert!(!leftover, "scratch key should not survive a replace");
}

#[tokio::test]
async fn empty_replace_clears_the_list_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::days(7));

    list.replace(&mut conn, &[pair("rk1", CID_1)], Utc::now())
        .await
        .expect("seed");
    list.replace(&mut conn, &[], Utc::now())
        .await
        .expect("clear");

    let read = list.read(&mut conn).await.expect("read");
    assert!(read.is_empty());
    let exists: bool = conn.exists("v3-recency-7d").await.expect("exists");
    assert!(!exists, "an empty list leaves no key");
}

#[tokio::test]
async fn distinct_names_do_not_collide_docker() {
    let (_container, mut conn) = start_redis().await;
    let short = PublishedList::new(Order::Recency, Limit::hours(48));
    let long = PublishedList::new(Order::Recency, Limit::days(7));

    let short_pairs = vec![pair("rk1", CID_1)];
    let long_pairs = vec![pair("rk2", CID_2), pair("rk3", CID_3)];
    short
        .replace(&mut conn, &short_pairs, Utc::now())
        .await
        .expect("short");
    long.replace(&mut conn, &long_pairs, Utc::now())
        .await
        .expect("long");

    assert_eq!(
        short.read(&mut conn).await.expect("read short"),
        short_pairs
    );
    assert_eq!(long.read(&mut conn).await.expect("read long"), long_pairs);
}

#[tokio::test]
async fn readers_filter_missing_items_but_published_keeps_them_docker() {
    let container = Redis::default()
        .start()
        .await
        .expect("start redis container");
    let host = container.get_host().await.expect("get host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("get mapped port");
    let url = format!("redis://{host}:{port}");
    let mut conn = connect_with_retry(&url).await;

    let list = PublishedList::new(Order::Recency, Limit::hours(48));
    let present = pair("present", CID_1);
    let mut image_gone = pair("imagegone", CID_2);
    image_gone.image_url_exists = false;
    let mut skeet_gone = pair("skeetgone", CID_3);
    skeet_gone.skeet_id_exists = false;
    let pairs = vec![present, image_gone, skeet_gone];
    list.replace(&mut conn, &pairs, Utc::now())
        .await
        .expect("replace");

    let reader = RedisFeedSource::new(url, Order::Recency, Limit::hours(48));

    // The raw read keeps every item (appraise needs them all)...
    let (raw, _) = reader.published().await.expect("published");
    assert_eq!(raw.len(), 3, "published() is unfiltered");

    // ...but both reader views drop the items flagged missing.
    let skeleton = reader.skeleton(false).await.expect("skeleton");
    let skeleton_rkeys: Vec<String> = skeleton
        .skeet_ids
        .iter()
        .map(|s| s.rkey().as_str().to_string())
        .collect();
    assert_eq!(skeleton_rkeys, ["present"]);

    let images = reader.published_images().await.expect("published images");
    let image_rkeys: Vec<String> = images
        .images
        .iter()
        .map(|p| p.skeet_id.rkey().as_str().to_string())
        .collect();
    assert_eq!(image_rkeys, ["present"]);
}


#[tokio::test]
async fn list_statistics_roundtrip_and_absent_before_first_write_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Quality, Limit::days(7));

    // Absent before the first write — a reader on a fresh deploy sees None.
    assert_eq!(list.read_statistics(&mut conn).await.expect("read"), None);

    let stats = ListStatistics::new(
        Utc::now() - chrono::Duration::days(7),
        Utc::now(),
        400_000,
        46,
        44,
    );
    list.write_statistics(&mut conn, &stats)
        .await
        .expect("write");
    assert_eq!(
        list.read_statistics(&mut conn).await.expect("read"),
        Some(stats)
    );

    // It lives under the list's version-prefixed companion key.
    let exists: bool = conn.exists("v3-quality-7d:statistics").await.expect("exists");
    assert!(exists);
}

#[tokio::test]
async fn published_list_catalog_roundtrips_as_a_set_docker() {
    let (_container, mut conn) = start_redis().await;

    // Empty before the publisher advertises anything.
    assert!(
        PublishedListCatalog::read(&mut conn)
            .await
            .expect("read")
            .is_empty()
    );

    let lists = vec![
        PublishedList::new(Order::Quality, Limit::weeks(4)),
        PublishedList::new(Order::Recency, Limit::hours(48)),
        PublishedList::new(Order::Quality, Limit::years(1)),
    ];
    PublishedListCatalog::write(&mut conn, &lists)
        .await
        .expect("write");

    // Membership matches regardless of order (it's a set).
    let mut read: Vec<String> = PublishedListCatalog::read(&mut conn)
        .await
        .expect("read")
        .iter()
        .map(PublishedList::name)
        .collect();
    read.sort();
    let mut expected: Vec<String> = lists.iter().map(PublishedList::name).collect();
    expected.sort();
    assert_eq!(read, expected);

    // Members are the published-list keys (version-prefixed).
    assert!(expected.iter().all(|n| n.starts_with("v3-")));

    // The catalog itself lives under the version-prefixed key.
    let exists: bool = conn.exists("v3-feed-catalog").await.expect("exists");
    assert!(exists);

    // An empty write clears it.
    PublishedListCatalog::write(&mut conn, &[])
        .await
        .expect("clear");
    assert!(
        PublishedListCatalog::read(&mut conn)
            .await
            .expect("read")
            .is_empty()
    );
    let exists: bool = conn.exists("v3-feed-catalog").await.expect("exists");
    assert!(!exists, "an empty catalog leaves no key");
}

#[tokio::test]
async fn refreshed_at_is_recorded_on_replace_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::hours(48));

    // Absent before the first publish.
    assert!(
        list.refreshed_at(&mut conn)
            .await
            .expect("read ts")
            .is_none()
    );

    let when = Utc::now();
    list.replace(&mut conn, &[pair("rk1", CID_1)], when)
        .await
        .expect("replace");

    let got = list
        .refreshed_at(&mut conn)
        .await
        .expect("read ts")
        .expect("timestamp present");
    assert_eq!(got.timestamp(), when.timestamp());
}
