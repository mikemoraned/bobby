//! Integration tests for the published-list redis schema against a real redis.
//!
//! A testcontainers redis is started and the `PublishedList` read/write helpers
//! are exercised end-to-end — round-trip, atomic replacement, and empty-clears.
//! This is the in-memory safety net to run before publishing to real Upstash.
//!
//! Requires Docker; the `_docker`-suffixed names are filtered out by the
//! `no-docker` nextest profile (`just test-no-docker`).

use deadpool_redis::redis::{self, AsyncCommands};
use skeet_publish::{ImageUrl, Limit, Order, PublishedList, PublishedPair};
use skeet_store::SkeetId;
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};

async fn start_redis() -> (ContainerAsync<Redis>, redis::aio::MultiplexedConnection) {
    let container = Redis::default().start().await.expect("start redis container");
    let host = container.get_host().await.expect("get host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("get mapped port");
    let client =
        redis::Client::open(format!("redis://{host}:{port}")).expect("open redis client");
    let conn = client
        .get_multiplexed_async_connection()
        .await
        .expect("connect to redis");
    (container, conn)
}

fn pair(rkey: &str, cid: &str) -> PublishedPair {
    PublishedPair {
        image_url: ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{cid}@jpeg"
        ))
        .expect("valid url"),
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse::<SkeetId>()
            .expect("valid skeet id"),
    }
}

#[tokio::test]
async fn write_then_read_roundtrips_in_order_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::hours(48));

    let pairs = vec![pair("rk1", "cidone"), pair("rk2", "cidtwo"), pair("rk3", "cidthree")];
    list.replace(&mut conn, &pairs).await.expect("replace");

    let read = list.read(&mut conn).await.expect("read");
    assert_eq!(read, pairs, "read back the same pairs in the same order");

    // It really lives under the {order}-{limit} key.
    let exists: bool = conn.exists("recency-48h").await.expect("exists");
    assert!(exists);
}

#[tokio::test]
async fn replace_swaps_atomically_leaving_no_remnants_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::hours(48));

    let first = vec![pair("rk1", "cidone"), pair("rk2", "cidtwo")];
    list.replace(&mut conn, &first).await.expect("first replace");

    // A shorter second list must fully overwrite the first — no stale tail.
    let second = vec![pair("rk9", "cidnine")];
    list.replace(&mut conn, &second).await.expect("second replace");

    let read = list.read(&mut conn).await.expect("read");
    assert_eq!(read, second);

    // The scratch key used during the swap is gone.
    let leftover: bool = conn.exists("recency-48h:building").await.expect("exists");
    assert!(!leftover, "scratch key should not survive a replace");
}

#[tokio::test]
async fn empty_replace_clears_the_list_docker() {
    let (_container, mut conn) = start_redis().await;
    let list = PublishedList::new(Order::Recency, Limit::days(7));

    list.replace(&mut conn, &[pair("rk1", "cidone")])
        .await
        .expect("seed");
    list.replace(&mut conn, &[]).await.expect("clear");

    let read = list.read(&mut conn).await.expect("read");
    assert!(read.is_empty());
    let exists: bool = conn.exists("recency-7d").await.expect("exists");
    assert!(!exists, "an empty list leaves no key");
}

#[tokio::test]
async fn distinct_names_do_not_collide_docker() {
    let (_container, mut conn) = start_redis().await;
    let short = PublishedList::new(Order::Recency, Limit::hours(48));
    let long = PublishedList::new(Order::Recency, Limit::days(7));

    let short_pairs = vec![pair("rk1", "cidone")];
    let long_pairs = vec![pair("rk2", "cidtwo"), pair("rk3", "cidthree")];
    short.replace(&mut conn, &short_pairs).await.expect("short");
    long.replace(&mut conn, &long_pairs).await.expect("long");

    assert_eq!(short.read(&mut conn).await.expect("read short"), short_pairs);
    assert_eq!(long.read(&mut conn).await.expect("read long"), long_pairs);
}
