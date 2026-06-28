//! Integration tests that hit a running skeet-feed server via HTTP.
//!
//! When `TEST_BASE_URL` is set (e.g. `end_to_end_test_feed_staging`), tests hit
//! that URL and need no local infrastructure. Otherwise each test spawns its own
//! `skeet-feed` subprocess on a free port, pointed at a fresh testcontainers
//! redis (skeet-feed is storeless — its only input is the published list). The
//! `TestServer` Drop guard kills the child; the container stops on drop. nextest
//! runs each test in its own process, so a single shared server isn't viable.
//!
//! The local path needs Docker, so the test names are `_docker`-suffixed (skipped
//! by `just test-no-docker`); the `TEST_BASE_URL` staging path skips the
//! container entirely.
//!
//! Gated behind the `integ` feature so `just test` doesn't compile them.

#![cfg(feature = "integ")]

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bluesky::ImageUrl;
use chrono::Utc;
use shared::{BlueskyCid, ImageId, SkeetId};
use skeet_publish::{
    Limit, ListStatistics, Order, PublishedImage, PublishedList, PublishedListCatalog,
};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};

mod common;

struct TestServer {
    child: Option<Child>,
    _redis: Option<ContainerAsync<Redis>>,
    url: String,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn spawn_server() -> TestServer {
    if let Ok(url) = std::env::var("TEST_BASE_URL") {
        reqwest::get(&url)
            .await
            .unwrap_or_else(|e| panic!("server not reachable at {url}: {e}"));
        return TestServer {
            child: None,
            _redis: None,
            url,
        };
    }
    spawn_local_server().await.0
}

/// Spawn a local `skeet-feed` subprocess against a fresh testcontainers redis,
/// returning the server and the redis URL so a test can seed published lists.
/// Always local (needs Docker) — unlike [`spawn_server`], it ignores
/// `TEST_BASE_URL`.
async fn spawn_local_server() -> (TestServer, String) {
    let container = Redis::default()
        .start()
        .await
        .expect("start redis container");
    let host = container.get_host().await.expect("redis host");
    let redis_port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("redis port");
    let redis_url = format!("redis://{host}:{redis_port}");
    wait_redis_ready(&redis_url).await;

    let port = pick_free_port();
    let bind = format!("127.0.0.1:{port}");
    let hostname = format!("localhost:{port}");
    let publisher_did = format!("did:web:localhost:{port}");
    let bin = env!("CARGO_BIN_EXE_skeet-feed");
    let child = Command::new(bin)
        .args([
            "--bind",
            &bind,
            "--hostname",
            &hostname,
            "--publisher-did",
            &publisher_did,
            "--redis-publish-url",
            &redis_url,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn skeet-feed");

    let url = format!("http://127.0.0.1:{port}");
    let server = TestServer {
        child: Some(child),
        _redis: Some(container),
        url: url.clone(),
    };
    for _ in 0..30 {
        if reqwest::get(&url).await.is_ok() {
            return (server, redis_url);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("local skeet-feed server failed to become reachable at {url} within 15s");
}

/// Wait until the testcontainers redis actually answers (Docker's host
/// port-forward can refuse briefly after the container reports ready).
async fn wait_redis_ready(url: &str) {
    for _ in 0..100 {
        if skeet_publish::connect(url).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("redis container not reachable at {url} within 10s");
}

fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

async fn discover_feed_uri(client: &reqwest::Client, base: &str) -> String {
    let resp = client
        .get(format!("{base}/xrpc/app.bsky.feed.describeFeedGenerator"))
        .send()
        .await
        .expect("describeFeedGenerator request");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("valid json");
    let feeds = body["feeds"].as_array().expect("feeds array");
    assert!(
        !feeds.is_empty(),
        "server should advertise at least one feed"
    );
    feeds[0]["uri"]
        .as_str()
        .expect("feed uri is a string")
        .to_string()
}

#[tokio::test]
async fn describe_feed_generator_returns_feed_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/xrpc/app.bsky.feed.describeFeedGenerator"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("valid json");
    let feeds = body["feeds"].as_array().expect("feeds array");
    assert!(!feeds.is_empty());

    let uri = feeds[0]["uri"].as_str().expect("feed uri string");
    assert!(
        uri.starts_with("at://"),
        "feed URI should be an AT-URI, got: {uri}"
    );
    assert!(
        uri.contains("app.bsky.feed.generator/"),
        "feed URI should contain generator path, got: {uri}"
    );
}

#[tokio::test]
async fn did_document_is_valid_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/.well-known/did.json"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("valid json");
    assert!(body["id"].is_string(), "DID document should have an id");
    assert!(
        body["service"].is_array(),
        "DID document should have a service array"
    );
}

#[tokio::test]
async fn get_feed_skeleton_with_discovered_uri_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();
    let feed_uri = discover_feed_uri(&client, &base).await;

    let resp = client
        .get(format!(
            "{base}/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"
        ))
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status(),
        200,
        "getFeedSkeleton should succeed with the discovered feed URI"
    );

    let body: serde_json::Value = resp.json().await.expect("valid json");
    assert!(body["feed"].is_array(), "response should have a feed array");
}

#[tokio::test]
async fn get_feed_skeleton_rejects_wrong_uri_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{base}/xrpc/app.bsky.feed.getFeedSkeleton?feed=at://wrong/app.bsky.feed.generator/bogus"
        ))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 400);

    let body: serde_json::Value = resp.json().await.expect("valid json");
    assert_eq!(body["error"], "UnknownFeed");
}

/// The feed-side preferred list. When this is empty/absent, the fallback should
/// degrade to successively older same-order lists.
const FEED_PREFERRED: (Order, Limit) = (Order::Quality, Limit::hours(48));
/// The website-grid preferred list (wider window than the feed's).
const GRID_PREFERRED: (Order, Limit) = (Order::Quality, Limit::weeks(4));

/// A known-valid base32 CIDv1. The tests assert on rkeys / post links, never on
/// CIDs, so every seeded image can share one valid CID.
const VALID_CID: &str = "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn published_image(rkey: &str) -> PublishedImage {
    PublishedImage::unprobed(
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{VALID_CID}@jpeg"
        ))
        .expect("valid url"),
        ImageId::V3(BlueskyCid::new(VALID_CID).expect("valid cid")),
        SkeetId::for_post("did:plc:abc", rkey),
    )
}

/// Seed the catalog with `specs`, write each populated list (one image with the
/// given rkey) plus that list's statistics — the state skeet-feed reads per
/// request.
async fn seed(redis_url: &str, specs: &[(Order, Limit)], populated: &[((Order, Limit), &str)]) {
    let mut conn = skeet_publish::connect(redis_url).await.expect("connect redis");

    let catalog: Vec<PublishedList> = specs
        .iter()
        .map(|&(order, limit)| PublishedList::new(order, limit))
        .collect();
    PublishedListCatalog::write(&mut conn, &catalog)
        .await
        .expect("write catalog");

    let now = Utc::now();
    for &((order, limit), rkey) in populated {
        let list = PublishedList::new(order, limit);
        list.replace(&mut conn, &[published_image(rkey)], now)
            .await
            .expect("replace list");
        list.write_statistics(
            &mut conn,
            &ListStatistics::new(now - limit.window(), now, 123_456, 1, 1),
        )
        .await
        .expect("write statistics");
    }
}

async fn feed_post_rkeys(client: &reqwest::Client, base: &str, feed_uri: &str) -> Vec<String> {
    let resp = client
        .get(format!(
            "{base}/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"
        ))
        .send()
        .await
        .expect("getFeedSkeleton request");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("valid json");
    body["feed"]
        .as_array()
        .expect("feed array")
        .iter()
        .map(|p| {
            p["post"]
                .as_str()
                .expect("post string")
                .rsplit('/')
                .next()
                .expect("rkey")
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn feed_and_homepage_fall_back_to_older_lists_when_preferred_empty_docker() {
    let (server, redis_url) = spawn_local_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    // Both preferred lists are advertised but empty; older same-order lists carry
    // the data: the feed should fall back from 48h to 7d, the grid from 4w to 1y.
    seed(
        &redis_url,
        &[
            FEED_PREFERRED,
            (Order::Quality, Limit::days(7)),
            GRID_PREFERRED,
            (Order::Quality, Limit::years(1)),
        ],
        &[
            ((Order::Quality, Limit::days(7)), "feed7d"),
            ((Order::Quality, Limit::years(1)), "grid1y"),
        ],
    )
    .await;

    let feed_uri = discover_feed_uri(&client, base).await;
    assert_eq!(
        feed_post_rkeys(&client, base, &feed_uri).await,
        vec!["feed7d".to_string()],
        "getFeedSkeleton should serve the older quality-7d list when quality-48h is empty"
    );

    let resp = client.get(format!("{base}/")).send().await.expect("GET /");
    assert_eq!(resp.status(), 200);
    let home = resp.text().await.expect("home body");
    assert!(
        home.contains("post/grid1y"),
        "homepage should serve the older quality-1y list when quality-4w is empty"
    );
    assert!(
        home.contains("123,456 images checked over the past year"),
        "the statistics banner should reflect the older list actually served during degradation"
    );
    assert!(
        home.contains("You've reached the end of the images found so far!"),
        "homepage should show the end-of-feed message once a next arrival can be predicted"
    );
    assert!(
        home.contains("class=\"js-countdown\""),
        "homepage should include the countdown spans the auto-reload script drives"
    );
}

/// Seed the grid's preferred window with three candidates, one of which the
/// publisher's probe found deleted (image gone). The published list — and the
/// recorded `found` — count all three; the live source drops the dead one, so the
/// grid renders two. This is the state in which the banner's "of which X match"
/// must still agree with the images actually shown.
async fn seed_grid_with_a_dead_candidate(redis_url: &str) {
    let mut conn = skeet_publish::connect(redis_url).await.expect("connect redis");
    let list = PublishedList::new(GRID_PREFERRED.0, GRID_PREFERRED.1);
    PublishedListCatalog::write(&mut conn, &[PublishedList::new(GRID_PREFERRED.0, GRID_PREFERRED.1)])
        .await
        .expect("write catalog");

    let mut dead = published_image("dead");
    dead.image_url_exists = false;
    let images = vec![published_image("live1"), published_image("live2"), dead];
    // `found` counts all candidates; `exists` only the live ones, mirroring what
    // the publisher records (the dead one is dropped by the feed's render filter).
    let found = images.len() as u64;
    let exists = images.iter().filter(|i| i.is_live()).count() as u64;
    let now = Utc::now();
    list.replace(&mut conn, &images, now)
        .await
        .expect("replace list");
    list.write_statistics(
        &mut conn,
        &ListStatistics::new(now - GRID_PREFERRED.1.window(), now, 1_000_000, found, exists),
    )
    .await
    .expect("write statistics");
}

/// Internal-consistency sanity check: the banner's "of which X match" figure must
/// equal the number of images the grid renders. Runs locally (seeding a served
/// list whose published `found` exceeds the live, shown count) and against
/// staging/production via `TEST_BASE_URL` (over whatever real data is live).
#[tokio::test]
async fn homepage_banner_match_count_equals_grid_image_count_docker() {
    let client = reqwest::Client::new();
    let server = if std::env::var("TEST_BASE_URL").is_ok() {
        spawn_server().await
    } else {
        let (server, redis_url) = spawn_local_server().await;
        seed_grid_with_a_dead_candidate(&redis_url).await;
        server
    };
    let base = &server.url;

    let resp = client.get(format!("{base}/")).send().await.expect("GET /");
    assert_eq!(resp.status(), 200);
    let home = resp.text().await.expect("home body");

    let (claimed, shown) = common::banner_count_and_grid_size(&home);
    assert_eq!(
        claimed, shown,
        "banner claims {claimed} match but the grid renders {shown} images"
    );
}

#[tokio::test]
async fn feed_serves_preferred_list_when_populated_docker() {
    let (server, redis_url) = spawn_local_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    // Both the preferred 48h and the older 7d are populated; the preferred wins.
    seed(
        &redis_url,
        &[FEED_PREFERRED, (Order::Quality, Limit::days(7))],
        &[
            (FEED_PREFERRED, "feed48h"),
            ((Order::Quality, Limit::days(7)), "feed7d"),
        ],
    )
    .await;

    let feed_uri = discover_feed_uri(&client, base).await;
    assert_eq!(
        feed_post_rkeys(&client, base, &feed_uri).await,
        vec!["feed48h".to_string()],
        "getFeedSkeleton should serve the preferred quality-48h list when it is populated"
    );
}
