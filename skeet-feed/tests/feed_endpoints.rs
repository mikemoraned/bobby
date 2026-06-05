#![cfg(feature = "test")]

//! Handler-level tests for the feed endpoints, driven by a stub `FeedSource`.
//!
//! `skeet-feed` is storeless: it serves whatever the published list contains.
//! The visibility/scoring/ordering policy lives in `skeet-publish` (and is tested
//! there), so these tests only cover the HTTP surface — did.json,
//! describeFeedGenerator, and that `getFeedSkeleton` serves the source's skeet
//! ids, applies `max_entries`, sets `Last-Modified`, and rejects unknown feeds.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cot::test::Client;
use skeet_feed::FeedSourceLayer;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_publish::{FeedSkeleton, FeedSource, FeedSourceError};
use skeet_store::SkeetId;

/// A `FeedSource` that returns a fixed skeleton — stands in for the redis-backed
/// `RedisFeedSource` so the handler tests need no redis.
struct StubFeedSource {
    skeet_ids: Vec<SkeetId>,
    refreshed_at: Option<DateTime<Utc>>,
}

#[async_trait]
impl FeedSource for StubFeedSource {
    async fn skeleton(&self, _force_refresh: bool) -> Result<FeedSkeleton, FeedSourceError> {
        Ok(FeedSkeleton {
            skeet_ids: self.skeet_ids.clone(),
            refreshed_at: self.refreshed_at,
        })
    }
}

fn test_params() -> FeedParams {
    FeedParams {
        hostname: "test.example.com".to_string(),
        publisher_did: "did:web:test.example.com".to_string(),
        feed_name: "bobby-dev".to_string(),
        max_entries: 10,
    }
}

fn skeet_id(rkey: &str) -> SkeetId {
    format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
        .parse()
        .expect("valid skeet id")
}

async fn client_for(params: FeedParams, source: StubFeedSource) -> Client {
    let feed_source: Arc<dyn FeedSource> = Arc::new(source);
    let project = FeedProject {
        feed_source_layer: FeedSourceLayer::new(feed_source),
        feed_config_layer: FeedConfigLayer::new(params),
    };
    Client::new(project).await
}

async fn get_body(client: &mut Client, path: &str) -> (u16, String) {
    let response = client.get(path).await.expect("GET request");
    let status = response.status().as_u16();
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
    (
        status,
        String::from_utf8(body_bytes.to_vec()).expect("valid utf8"),
    )
}

async fn feed_posts(client: &mut Client, feed_uri: &str) -> Vec<String> {
    let (status, body) = get_body(
        client,
        &format!("/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"),
    )
    .await;
    assert_eq!(status, 200);
    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    resp["feed"]
        .as_array()
        .expect("feed is array")
        .iter()
        .map(|p| p["post"].as_str().expect("post string").to_string())
        .collect()
}

#[tokio::test]
async fn did_document_returns_valid_json() {
    let source = StubFeedSource {
        skeet_ids: vec![],
        refreshed_at: None,
    };
    let mut client = client_for(test_params(), source).await;

    let (status, body) = get_body(&mut client, "/.well-known/did.json").await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert!(json["id"].is_string());
    assert!(json["service"].is_array());
}

#[tokio::test]
async fn describe_returns_feed_list() {
    let source = StubFeedSource {
        skeet_ids: vec![],
        refreshed_at: None,
    };
    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(params, source).await;

    let (status, body) = get_body(&mut client, "/xrpc/app.bsky.feed.describeFeedGenerator").await;
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let feeds = json["feeds"].as_array().expect("feeds array");
    assert_eq!(feeds[0]["uri"].as_str().expect("uri"), feed_uri);
}

#[tokio::test]
async fn serves_the_sources_skeet_ids_in_order() {
    let source = StubFeedSource {
        skeet_ids: vec![skeet_id("a"), skeet_id("b"), skeet_id("c")],
        refreshed_at: None,
    };
    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(params, source).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(
        posts,
        vec![
            skeet_id("a").to_string(),
            skeet_id("b").to_string(),
            skeet_id("c").to_string(),
        ]
    );
}

#[tokio::test]
async fn returns_empty_feed_when_source_is_empty() {
    let source = StubFeedSource {
        skeet_ids: vec![],
        refreshed_at: None,
    };
    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(params, source).await;

    assert!(feed_posts(&mut client, &feed_uri).await.is_empty());
}

#[tokio::test]
async fn applies_max_entries_limit() {
    let source = StubFeedSource {
        skeet_ids: (0..5).map(|i| skeet_id(&format!("s{i}"))).collect(),
        refreshed_at: None,
    };
    let mut params = test_params();
    params.max_entries = 2;
    let feed_uri = params.feed_uri();
    let mut client = client_for(params, source).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(posts.len(), 2, "feed should cap at max_entries");
}

#[tokio::test]
async fn rejects_unknown_feed() {
    let source = StubFeedSource {
        skeet_ids: vec![],
        refreshed_at: None,
    };
    let mut client = client_for(test_params(), source).await;

    let (status, body) = get_body(
        &mut client,
        "/xrpc/app.bsky.feed.getFeedSkeleton?feed=at://wrong/app.bsky.feed.generator/bogus",
    )
    .await;
    assert_eq!(status, 400);
    let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(json["error"], "UnknownFeed");
}

#[tokio::test]
async fn feed_skeleton_includes_last_modified_header() {
    let source = StubFeedSource {
        skeet_ids: vec![skeet_id("lm1")],
        refreshed_at: Some(Utc::now()),
    };
    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(params, source).await;

    let response = client
        .get(&format!(
            "/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"
        ))
        .await
        .expect("GET feed skeleton");
    assert_eq!(response.status().as_u16(), 200);
    assert!(
        response.headers().get("last-modified").is_some(),
        "feed skeleton should include Last-Modified header"
    );
}
