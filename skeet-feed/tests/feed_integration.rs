//! Integration tests that hit a running skeet-feed server via HTTP.
//!
//! These require a live server — start one first with e.g. `just feed-r2`,
//! then run with `just integ_test_feed`.
//!
//! Gated behind the `integ` feature so `just test` doesn't compile them.

#![cfg(feature = "integ")]

async fn assert_server_available() -> String {
    let url =
        std::env::var("TEST_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    reqwest::get(&url)
        .await
        .unwrap_or_else(|e| panic!("server not reachable at {url}: {e}"));
    url
}

async fn discover_feed_uri(client: &reqwest::Client, base: &str) -> String {
    let resp = client
        .get(format!(
            "{base}/xrpc/app.bsky.feed.describeFeedGenerator"
        ))
        .send()
        .await
        .expect("describeFeedGenerator request");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("valid json");
    let feeds = body["feeds"].as_array().expect("feeds array");
    assert!(!feeds.is_empty(), "server should advertise at least one feed");
    feeds[0]["uri"]
        .as_str()
        .expect("feed uri is a string")
        .to_string()
}

#[tokio::test]
async fn describe_feed_generator_returns_feed() {
    let base = assert_server_available().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{base}/xrpc/app.bsky.feed.describeFeedGenerator"
        ))
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
async fn did_document_is_valid() {
    let base = assert_server_available().await;
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
async fn get_feed_skeleton_with_discovered_uri() {
    let base = assert_server_available().await;
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
    assert!(
        body["feed"].is_array(),
        "response should have a feed array"
    );
}

#[tokio::test]
async fn get_feed_skeleton_rejects_wrong_uri() {
    let base = assert_server_available().await;
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
