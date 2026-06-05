#![cfg(feature = "test")]

use std::sync::Arc;

use chrono::Utc;
use cot::test::Client;
use shared::{Appraiser, Band, ImageId};
use skeet_feed::FeedSourceLayer;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_publish::{FeedCache, FeedSource, LiveFeedSource};
use skeet_store::test_utils::{make_record, make_record_at, open_temp_store};
use skeet_store::{DiscoveredAt, ModelVersion, Score, SkeetId, SkeetStore};
use test_support::test_models;

/// Default cache window for the library-path fixture (`max_age_hours` is a
/// `FeedCache` concern, not part of the feed's serving config).
const DEFAULT_MAX_AGE_HOURS: u64 = 48;

fn test_params() -> FeedParams {
    FeedParams {
        hostname: "test.example.com".to_string(),
        publisher_did: "did:web:test.example.com".to_string(),
        feed_name: "bobby-dev".to_string(),
        max_entries: 10,
    }
}

async fn client_for(store: SkeetStore, params: FeedParams) -> Client {
    client_for_with_age(store, params, DEFAULT_MAX_AGE_HOURS).await
}

async fn client_for_with_age(store: SkeetStore, params: FeedParams, max_age_hours: u64) -> Client {
    let store = Arc::new(store);
    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        test_models(),
        params.max_entries,
        max_age_hours,
    ));
    let feed_source: Arc<dyn FeedSource> = Arc::new(LiveFeedSource::new(Arc::clone(&cache)));
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

/// Add a record and score it, returning the typed (skeet_id, image_id).
async fn seed_scored(store: &SkeetStore, suffix: &str, r: u8, score: f32) -> (SkeetId, ImageId) {
    let record = make_record(suffix, r, 0, 0);
    let image_id = record.image_id.clone();
    let skeet_id = record.skeet_id.clone();
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &record.image_id,
            &Score::new(score).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");
    (skeet_id, image_id)
}

/// Fetch the feed skeleton with `Cache-Control: no-cache` to guarantee fresh data.
async fn feed_posts(client: &mut Client, feed_uri: &str) -> Vec<String> {
    let request = cot::http::Request::builder()
        .uri(format!(
            "/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"
        ))
        .header("cache-control", "no-cache")
        .body(cot::Body::empty())
        .expect("build request");
    let response = client.request(request).await.expect("GET feed skeleton");
    assert_eq!(response.status().as_u16(), 200);
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("valid utf8");
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
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let mut client = client_for(store, test_params()).await;

    let (status, body) = get_body(&mut client, "/.well-known/did.json").await;
    assert_eq!(status, 200);

    let doc: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(doc["id"], "did:web:test.example.com");
    assert_eq!(doc["service"][0]["id"], "#bsky_fg");
    assert_eq!(doc["service"][0]["type"], "BskyFeedGenerator");
    assert_eq!(
        doc["service"][0]["serviceEndpoint"],
        "https://test.example.com"
    );
}

#[tokio::test]
async fn describe_returns_feed_list() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let mut client = client_for(store, test_params()).await;

    let (status, body) = get_body(&mut client, "/xrpc/app.bsky.feed.describeFeedGenerator").await;
    assert_eq!(status, 200);

    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(resp["did"], "did:web:test.example.com");
    assert_eq!(
        resp["feeds"][0]["uri"],
        "at://did:web:test.example.com/app.bsky.feed.generator/bobby-dev"
    );
}

#[tokio::test]
async fn returns_empty_feed_when_no_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(posts.is_empty());
}

#[tokio::test]
async fn returns_scored_posts_above_threshold() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "good1", 0, 0.85).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(posts, vec![skeet_id.to_string()]);
}

#[tokio::test]
async fn excludes_posts_below_threshold() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    seed_scored(&store, "low1", 255, 0.2).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(posts.is_empty());
}

#[tokio::test]
async fn rejects_unknown_feed() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let mut client = client_for(store, test_params()).await;

    let (status, body) = get_body(
        &mut client,
        "/xrpc/app.bsky.feed.getFeedSkeleton?feed=at://unknown/app.bsky.feed.generator/nope",
    )
    .await;
    assert_eq!(status, 400);

    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(resp["error"], "UnknownFeed");
}

/// Recent posts with moderate scores should appear in the feed even when older
/// posts have higher scores. The feed must filter by max_age_hours first, then
/// rank by score — not take the global top-N by score and then filter by age.
#[tokio::test]
async fn prefers_recent_posts_over_old_high_scores() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let mv = ModelVersion::from("test");

    let mut params = test_params();
    params.max_entries = 2;
    let max_age_hours = 24;

    // Create 3 old posts (outside max_age_hours) with very high scores
    let three_days_ago = Utc::now() - chrono::Duration::hours(72);
    for i in 0..3 {
        let record = make_record_at(
            &format!("old{i}"),
            (100 + i) as u8,
            0,
            0,
            DiscoveredAt::new(three_days_ago),
        );
        let image_id = record.image_id.clone();
        store.add(&record).await.expect("add record");
        store
            .upsert_score(&image_id, &Score::new(0.99).expect("valid"), &mv)
            .await
            .expect("upsert score");
    }

    // Create 1 recent post with a moderate score (above threshold)
    let recent = make_record("recent1", 0, 200, 0);
    let recent_id = recent.image_id.clone();
    store.add(&recent).await.expect("add record");
    store
        .upsert_score(&recent_id, &Score::new(0.7).expect("valid"), &mv)
        .await
        .expect("upsert score");

    let feed_uri = params.feed_uri();
    let mut client = client_for_with_age(store, params, max_age_hours).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(
        posts,
        vec!["at://did:plc:abc/app.bsky.feed.post/recent1"],
        "the recent post should appear; old posts should be filtered out"
    );
}

#[tokio::test]
async fn skeet_visible_by_default_when_high_scored() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "vis1", 10, 0.85).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(posts, vec![skeet_id.to_string()]);
}

// ─── Appraisal visibility (cross-service) ──────────────────────
//
// Appraisals are made in `skeet-appraise`; the feed only *reads* the resulting
// visibility. These cases seed appraisals directly via the store (the same
// rows the appraise UI would write) and assert against `getFeedSkeleton`.

#[tokio::test]
async fn manually_demoting_skeet_hides_it() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "demote1", 10, 0.85).await;
    store
        .set_skeet_band(&skeet_id, Band::Low, &Appraiser::LocalAdmin)
        .await
        .expect("set skeet band");

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(posts.is_empty(), "demoted skeet should be hidden");
}

#[tokio::test]
async fn manually_demoting_image_hides_skeet() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "imgdemote1", 10, 0.85).await;
    store
        .set_image_band(&image_id, Band::Low, &Appraiser::LocalAdmin)
        .await
        .expect("set image band");

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(
        posts.is_empty(),
        "skeet with demoted image should be hidden"
    );
}

#[tokio::test]
async fn promoting_skeet_alone_not_enough_when_image_is_low() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "promote1", 10, 0.1).await;

    // Score 0.1 → Band::Low → not visible by default.
    // Promote only the skeet — image is still Low from its score.
    // Lowest band across skeet + images determines visibility.
    store
        .set_skeet_band(&skeet_id, Band::HighQuality, &Appraiser::LocalAdmin)
        .await
        .expect("set skeet band");

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(
        posts.is_empty(),
        "low image should still block promoted skeet"
    );
}

#[tokio::test]
async fn promoting_skeet_and_image_shows_low_scored_skeet() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, image_id) = seed_scored(&store, "promote2", 10, 0.1).await;

    // Both skeet and image must be promoted for the skeet to appear.
    store
        .set_skeet_band(&skeet_id, Band::HighQuality, &Appraiser::LocalAdmin)
        .await
        .expect("set skeet band");
    store
        .set_image_band(&image_id, Band::HighQuality, &Appraiser::LocalAdmin)
        .await
        .expect("set image band");

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(
        posts,
        vec![skeet_id.to_string()],
        "fully promoted skeet should be visible"
    );
}

// ─── Feed skeleton caching ─────────────────────────────────────

#[tokio::test]
async fn feed_skeleton_includes_last_modified_header() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    seed_scored(&store, "lm1", 10, 0.85).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let request = cot::http::Request::builder()
        .uri(format!(
            "/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"
        ))
        .header("cache-control", "no-cache")
        .body(cot::Body::empty())
        .expect("build request");
    let response = client.request(request).await.expect("GET feed skeleton");
    assert_eq!(response.status().as_u16(), 200);
    assert!(
        response.headers().get("last-modified").is_some(),
        "feed skeleton should include Last-Modified header"
    );
}
