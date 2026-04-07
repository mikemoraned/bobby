#![cfg(feature = "test")]

use chrono::Utc;
use cot::test::Client;
use image::{DynamicImage, ImageBuffer, Rgba};
use skeet_feed::StoreLayer;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_store::{
    DiscoveredAt, ImageId, ImageRecord, ModelVersion, OriginalAt, Score, SkeetStore, Zone,
};

fn test_image() -> DynamicImage {
    DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255])))
}

fn test_params() -> FeedParams {
    FeedParams {
        hostname: "test.example.com".to_string(),
        publisher_did: "did:web:test.example.com".to_string(),
        feed_name: "bobby-dev".to_string(),
        max_entries: 10,
        min_score: 0.5,
        max_age_hours: 48,
    }
}

fn make_record(suffix: &str, r: u8, g: u8, b: u8) -> ImageRecord {
    make_record_at(suffix, r, g, b, DiscoveredAt::now())
}

fn make_record_at(
    suffix: &str,
    r: u8,
    g: u8,
    b: u8,
    discovered_at: DiscoveredAt,
) -> ImageRecord {
    let img = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([r, g, b, 255])));
    ImageRecord {
        image_id: ImageId::from_image(&img),
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{suffix}")
            .parse()
            .expect("valid AT URI"),
        image: img,
        discovered_at,
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    }
}

async fn open_temp_store(dir: &tempfile::TempDir) -> SkeetStore {
    SkeetStore::open(dir.path().to_str().expect("valid path"), vec![], None)
        .await
        .expect("open store")
}

async fn client_for(store: SkeetStore, params: FeedParams) -> Client {
    let project = FeedProject {
        store_layer: StoreLayer::new(store),
        feed_config_layer: FeedConfigLayer::new(params),
    };
    Client::new(project).await
}

async fn get_body(client: &mut Client, path: &str) -> (u16, String) {
    let response = client.get(path).await.expect("GET request");
    let status = response.status().as_u16();
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
    (status, String::from_utf8(body_bytes.to_vec()).expect("valid utf8"))
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

    let (status, body) = get_body(
        &mut client,
        "/xrpc/app.bsky.feed.describeFeedGenerator",
    )
    .await;
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

    let (status, body) = get_body(
        &mut client,
        &format!("/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"),
    )
    .await;
    assert_eq!(status, 200);

    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert!(resp["feed"].as_array().expect("feed is array").is_empty());
}

#[tokio::test]
async fn returns_scored_posts_above_threshold() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("good1", 0, 255, 0);
    let image_id = record.image_id.clone();
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &image_id,
            &Score::new(0.85).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let (status, body) = get_body(
        &mut client,
        &format!("/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"),
    )
    .await;
    assert_eq!(status, 200);

    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let feed = resp["feed"].as_array().expect("feed is array");
    assert_eq!(feed.len(), 1);
    assert_eq!(
        feed[0]["post"],
        "at://did:plc:abc/app.bsky.feed.post/good1"
    );
}

#[tokio::test]
async fn excludes_posts_below_threshold() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("low1", 255, 0, 0);
    let image_id = record.image_id.clone();
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &image_id,
            &Score::new(0.2).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    let (status, body) = get_body(
        &mut client,
        &format!("/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"),
    )
    .await;
    assert_eq!(status, 200);

    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert!(resp["feed"].as_array().expect("feed is array").is_empty());
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
    params.min_score = 0.5;
    params.max_age_hours = 24;

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
    let mut client = client_for(store, params).await;

    let (status, body) = get_body(
        &mut client,
        &format!("/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"),
    )
    .await;
    assert_eq!(status, 200);

    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let feed = resp["feed"].as_array().expect("feed is array");
    assert_eq!(
        feed.len(),
        1,
        "the recent post should appear; old posts should be filtered out"
    );
    assert_eq!(feed[0]["post"], "at://did:plc:abc/app.bsky.feed.post/recent1");
}
