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
use shared::{BlueskyCid, ImageId};
use bluesky::{Dimensions, ImageUrl};
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_feed::{FeedSourceLayer, PublishedImagesSourceLayer};
use skeet_publish::{
    FeedSkeleton, FeedSource, FeedSourceError, PublishedImage, PublishedImages,
    PublishedImagesSource,
};
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

/// A `PublishedImagesSource` returning a fixed set of images for the home page.
struct StubPublishedImagesSource {
    images: Vec<PublishedImage>,
}

#[async_trait]
impl PublishedImagesSource for StubPublishedImagesSource {
    async fn published_images(&self) -> Result<PublishedImages, FeedSourceError> {
        Ok(PublishedImages {
            images: self.images.clone(),
            refreshed_at: None,
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

fn project_for(
    params: FeedParams,
    feed_source: Arc<dyn FeedSource>,
    images_source: Arc<dyn PublishedImagesSource>,
) -> FeedProject {
    FeedProject {
        feed_source_layer: FeedSourceLayer::new(feed_source),
        published_images_source_layer: PublishedImagesSourceLayer::new(images_source),
        feed_config_layer: FeedConfigLayer::new(params),
    }
}

async fn client_for(params: FeedParams, source: StubFeedSource) -> Client {
    let empty_images = Arc::new(StubPublishedImagesSource { images: vec![] });
    let project = project_for(params, Arc::new(source), empty_images);
    Client::new(project).await
}

async fn client_with_images(params: FeedParams, images: Vec<PublishedImage>) -> Client {
    let empty_feed = Arc::new(StubFeedSource {
        skeet_ids: vec![],
        refreshed_at: None,
    });
    let images_source = Arc::new(StubPublishedImagesSource { images });
    let project = project_for(params, empty_feed, images_source);
    Client::new(project).await
}

fn thumb_url(cid: &str) -> String {
    format!("https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{cid}@jpeg")
}

fn published_image(rkey: &str, cid: &str) -> PublishedImage {
    PublishedImage::unprobed(
        ImageUrl::new(thumb_url(cid)).expect("valid url"),
        ImageId::V3(BlueskyCid::new(cid).expect("valid cid")),
        skeet_id(rkey),
    )
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
async fn home_renders_grid_of_cards_in_order() {
    const CID_1: &str = "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CID_2: &str = "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    let images = vec![
        published_image("first", CID_1),
        published_image("second", CID_2),
    ];
    let mut client = client_with_images(test_params(), images).await;

    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);

    // Each card links to the skeet's Bluesky post and shows its CDN thumbnail.
    assert!(
        body.contains("https://bsky.app/profile/did:plc:abc/post/first"),
        "card should link to the first skeet's bsky post"
    );
    assert!(
        body.contains(&format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{CID_1}@jpeg"
        )),
        "card should show the first image's CDN thumbnail"
    );

    // Cards render in published order: "first" before "second".
    let first_at = body.find("post/first").expect("first card present");
    let second_at = body.find("post/second").expect("second card present");
    assert!(first_at < second_at, "cards should be in published order");
}

#[tokio::test]
async fn home_renders_empty_state_when_no_images() {
    let mut client = client_with_images(test_params(), vec![]).await;

    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(
        !body.contains("class=\"grid\""),
        "no grid should render when there are no images"
    );
}

#[tokio::test]
async fn home_sets_aspect_ratio_when_dimensions_known() {
    const CID: &str = "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    let mut image = published_image("p", CID);
    image.image_url_dimensions = Some(Dimensions {
        width: 800,
        height: 600,
    });
    let mut client = client_with_images(test_params(), vec![image]).await;

    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(
        body.contains(r#"style="aspect-ratio: 800/600""#),
        "card should carry the known aspect ratio"
    );
}

#[tokio::test]
async fn home_omits_aspect_ratio_when_dimensions_unknown() {
    const CID: &str = "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    // unprobed() leaves dimensions unknown → no aspect-ratio rendered.
    let mut client = client_with_images(test_params(), vec![published_image("p", CID)]).await;

    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(
        !body.contains("aspect-ratio"),
        "no aspect-ratio style when dimensions are unknown"
    );
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
