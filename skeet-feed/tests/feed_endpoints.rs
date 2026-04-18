#![cfg(feature = "test")]

use std::sync::Arc;

use chrono::Utc;
use cot::test::Client;
use shared::Appraiser;
use skeet_feed::auth_config::OAuthConfig;
use skeet_feed::{AppraiserLayer, FeedCacheLayer, OAuthConfigLayer, StartedAtLayer, StoreLayer};
use skeet_feed::feed_cache::FeedCache;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_store::test_utils::{make_record, make_record_at, open_temp_store};
use skeet_store::{DiscoveredAt, ModelVersion, Score, SkeetStore};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_params() -> FeedParams {
    FeedParams {
        hostname: "test.example.com".to_string(),
        publisher_did: "did:web:test.example.com".to_string(),
        feed_name: "bobby-dev".to_string(),
        max_entries: 10,
        max_age_hours: 48,
    }
}

async fn client_for(store: SkeetStore, params: FeedParams) -> Client {
    let store = Arc::new(store);
    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        params.max_entries,
        params.max_age_hours,
    ));
    let project = FeedProject {
        cache_layer: FeedCacheLayer::new(cache),
        feed_config_layer: FeedConfigLayer::new(params),
        store_layer: StoreLayer::from_shared(store),
        appraiser_layer: AppraiserLayer::new(Some(Arc::new(Appraiser::LocalAdmin))),
        oauth_config_layer: OAuthConfigLayer::new(None),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: None,
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
    assert_eq!(posts, vec![skeet_id]);
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

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(
        posts,
        vec!["at://did:plc:abc/app.bsky.feed.post/recent1"],
        "the recent post should appear; old posts should be filtered out"
    );
}

/// Helper: add a record and score it, returning (skeet_id string, image_id string).
async fn seed_scored(store: &SkeetStore, suffix: &str, r: u8, score: f32) -> (String, String) {
    let record = make_record(suffix, r, 0, 0);
    let image_id = record.image_id.to_string();
    let skeet_id = record.skeet_id.to_string();
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
        .uri(format!("/xrpc/app.bsky.feed.getFeedSkeleton?feed={feed_uri}"))
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

/// Appraise a skeet or image via the public HTTP endpoint.
async fn appraise(client: &mut Client, kind: &str, id: &str, band: &str) {
    let encoded_id = urlencoding::encode(id);
    let (status, _) = get_body(
        client,
        &format!("/admin/appraise/{kind}?band={band}&id={encoded_id}"),
    )
    .await;
    assert_eq!(status, 200, "appraise {kind} {id} as {band} should succeed");
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
    assert_eq!(posts, vec![skeet_id]);
}

#[tokio::test]
async fn manually_demoting_skeet_hides_it() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "demote1", 10, 0.85).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    appraise(&mut client, "skeet", &skeet_id, "Low").await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(posts.is_empty(), "demoted skeet should be hidden");
}

#[tokio::test]
async fn manually_demoting_image_hides_skeet() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "imgdemote1", 10, 0.85).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    appraise(&mut client, "image", &image_id, "Low").await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(posts.is_empty(), "skeet with demoted image should be hidden");
}

#[tokio::test]
async fn promoting_skeet_alone_not_enough_when_image_is_low() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "promote1", 10, 0.1).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    // Score 0.1 → Band::Low → not visible by default.
    // Promote only the skeet — image is still Low from its score.
    // Lowest band across skeet + images determines visibility.
    appraise(&mut client, "skeet", &skeet_id, "HighQuality").await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert!(posts.is_empty(), "low image should still block promoted skeet");
}

#[tokio::test]
async fn promoting_skeet_and_image_shows_low_scored_skeet() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, image_id) = seed_scored(&store, "promote2", 10, 0.1).await;

    let params = test_params();
    let feed_uri = params.feed_uri();
    let mut client = client_for(store, params).await;

    // Both skeet and image must be promoted for the skeet to appear.
    appraise(&mut client, "skeet", &skeet_id, "HighQuality").await;
    appraise(&mut client, "image", &image_id, "HighQuality").await;

    let posts = feed_posts(&mut client, &feed_uri).await;
    assert_eq!(posts, vec![skeet_id], "fully promoted skeet should be visible");
}

// ─── Static assets ──────────────────────────────────────────────

#[tokio::test]
async fn static_htmx_js_is_served() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let mut client = client_for(store, test_params()).await;

    let (status, body) = get_body(&mut client, "/static/htmx.min.js").await;
    assert_eq!(status, 200, "htmx.min.js should be served at /static/htmx.min.js");
    assert!(body.contains("htmx"), "response should contain htmx code");
}

// ─── Annotated image caching ────────────────────────────────────

#[tokio::test]
async fn annotated_image_returns_last_modified_header() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "img1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    let response = client
        .get(&format!("/skeet/{image_id}/annotated.png"))
        .await
        .expect("GET annotated image");
    assert_eq!(response.status().as_u16(), 200);
    assert!(
        response.headers().get("last-modified").is_some(),
        "response should include Last-Modified header"
    );
}

#[tokio::test]
async fn annotated_image_conditional_get_returns_304() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "img2", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    // First request — get the Last-Modified value
    let response = client
        .get(&format!("/skeet/{image_id}/annotated.png"))
        .await
        .expect("GET annotated image");
    assert_eq!(response.status().as_u16(), 200);
    let last_modified = response
        .headers()
        .get("last-modified")
        .expect("Last-Modified header")
        .to_str()
        .expect("valid header value")
        .to_string();

    // Second request with If-Modified-Since — should get 304
    let request = cot::http::Request::builder()
        .uri(format!("/skeet/{image_id}/annotated.png"))
        .header("if-modified-since", &last_modified)
        .body(cot::Body::empty())
        .expect("build request");
    let response = client.request(request).await.expect("conditional GET");
    assert_eq!(
        response.status().as_u16(),
        304,
        "should return 304 Not Modified when If-Modified-Since matches"
    );
}

// ─── Admin view tests ───────────────────────────────────────────

/// Extract the item IDs from `<td class="id">...</td>` cells in the HTML.
fn extract_item_ids(html: &str) -> Vec<String> {
    let tag = r#"<td class="id">"#;
    let mut ids = Vec::new();
    let mut start = 0;
    while let Some(pos) = html[start..].find(tag) {
        let begin = start + pos + tag.len();
        if let Some(end_offset) = html[begin..].find("</td>") {
            ids.push(html[begin..begin + end_offset].trim().to_string());
        }
        start = begin;
    }
    ids
}

/// Check whether an admin row's HTML contains a manual band tag with the given text.
fn row_has_manual_band(html: &str, band: &str) -> bool {
    // Manual band is in the 5th <td> — look for the band-tag span after "Manual Band" column
    html.contains(&format!(r#"<span class="band-tag {band}">{band}</span>"#))
}

/// Check whether an admin row's HTML shows "—" for the manual band (no manual appraisal).
fn row_has_no_manual_band(html: &str) -> bool {
    html.contains(r#"<span style="color:#999">—</span>"#)
}

/// Check that an admin row mentions the given appraiser.
fn row_has_appraiser(html: &str, appraiser: &str) -> bool {
    html.contains(&format!("by {appraiser}"))
}

/// Seed items at specific minute offsets (larger offset = older) and score them.
async fn seed_n_scored(store: &SkeetStore, n: usize) -> Vec<String> {
    let now = Utc::now();
    let mut skeet_ids = Vec::new();
    for i in 0..n {
        let suffix = format!("item{i}");
        let discovered = DiscoveredAt::new(now - chrono::Duration::minutes(i as i64));
        let record = make_record_at(&suffix, (10 + i) as u8, 0, 0, discovered);
        let image_id = record.image_id.clone();
        let skeet_id = record.skeet_id.to_string();
        store.add(&record).await.expect("add record");
        store
            .upsert_score(
                &image_id,
                &Score::new(0.85).expect("valid score"),
                &ModelVersion::from("test"),
            )
            .await
            .expect("upsert score");
        skeet_ids.push(skeet_id);
    }
    skeet_ids
}

#[tokio::test]
async fn admin_paging_returns_items_in_discovered_at_desc_order() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    // 12 items: item0 (newest) .. item11 (oldest). Page size is 10.
    let skeet_ids = seed_n_scored(&store, 12).await;

    let mut client = client_for(store, test_params()).await;

    // First page: 10 newest items (item0..item9)
    let (status, body) = get_body(&mut client, "/admin").await;
    assert_eq!(status, 200);
    let first_page_ids = extract_item_ids(&body);
    assert_eq!(first_page_ids.len(), 10);
    assert_eq!(first_page_ids, skeet_ids[..10]);

    // Extract cursor from the htmx load-more div
    let cursor_marker = r#"hx-get="/admin?view=skeet&cursor="#;
    let cursor_pos = body.find(cursor_marker).expect("should have a next-page cursor");
    let after = &body[cursor_pos + cursor_marker.len()..];
    let cursor_end = after.find('"').expect("cursor value ends with quote");
    let cursor = &after[..cursor_end];

    // Second page: remaining 2 items (item10, item11)
    let (status, body) = get_body(&mut client, &format!("/admin?cursor={cursor}")).await;
    assert_eq!(status, 200);
    let second_page_ids = extract_item_ids(&body);
    assert_eq!(second_page_ids.len(), 2);
    assert_eq!(second_page_ids, skeet_ids[10..]);

    // No more cursor
    assert!(
        !body.contains(cursor_marker),
        "should not have another next-page cursor"
    );
}

#[tokio::test]
async fn admin_set_manual_band_updates_row() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "band1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    // Before: no manual band
    let (_, body) = get_body(&mut client, "/admin").await;
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert_eq!(rows_html.len(), 1);
    assert!(row_has_no_manual_band(rows_html[0]));

    // Appraise via HTTP
    appraise(&mut client, "skeet", &skeet_id, "Low").await;

    // After: manual band shows "Low" with appraiser
    let (_, body) = get_body(&mut client, "/admin").await;
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert_eq!(rows_html.len(), 1);
    assert!(
        row_has_manual_band(rows_html[0], "Low"),
        "row should show manual band Low"
    );
    assert!(
        row_has_appraiser(rows_html[0], "local:admin"),
        "row should show appraiser"
    );
}

#[tokio::test]
async fn admin_clear_manual_band_reverts_to_automatic() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "clear1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    // Set a manual band
    appraise(&mut client, "skeet", &skeet_id, "Low").await;
    let (_, body) = get_body(&mut client, "/admin").await;
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert!(row_has_manual_band(rows_html[0], "Low"));

    // Clear it
    appraise(&mut client, "skeet", &skeet_id, "clear").await;

    // After clear: no manual band, reverted to automatic
    let (_, body) = get_body(&mut client, "/admin").await;
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert!(
        row_has_no_manual_band(rows_html[0]),
        "clearing should revert to automatic (no manual band)"
    );
}

// ─── Admin image view ──────────────────────────────────────────

#[tokio::test]
async fn admin_image_view_shows_image_ids() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "imgview1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    let (status, body) = get_body(&mut client, "/admin?view=image").await;
    assert_eq!(status, 200);
    let ids = extract_item_ids(&body);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], image_id, "image view should show image IDs");
}

#[tokio::test]
async fn admin_image_view_with_manual_band_shows_effective_band() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "imgeff1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    // Set a manual image band
    appraise(&mut client, "image", &image_id, "Low").await;

    // In image view, effective band should use image_effective_band(score, Some(Low)) = Low
    let (status, body) = get_body(&mut client, "/admin?view=image").await;
    assert_eq!(status, 200);
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert_eq!(rows_html.len(), 1);
    assert!(
        row_has_manual_band(rows_html[0], "Low"),
        "image view should show manual band Low"
    );
}

#[tokio::test]
async fn appraise_skeet_returns_row_html() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "approw1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    let encoded_id = urlencoding::encode(&skeet_id);
    let (status, body) = get_body(
        &mut client,
        &format!("/admin/appraise/skeet?band=Low&id={encoded_id}"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("<tr"), "appraise response should contain a table row");
    assert!(body.contains("Low"), "appraise response should show the new band");
}

#[tokio::test]
async fn appraise_image_returns_row_html() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "appimg1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    let encoded_id = urlencoding::encode(&image_id);
    let (status, body) = get_body(
        &mut client,
        &format!("/admin/appraise/image?band=HighQuality&id={encoded_id}"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(body.contains("<tr"), "appraise response should contain a table row");
    assert!(
        body.contains("HighQuality"),
        "appraise response should show the new band"
    );
}

// ─── Home page ─────────────────────────────────────────────────

#[tokio::test]
async fn home_page_shows_scored_entries() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    seed_scored(&store, "home1", 10, 0.85).await;

    let mut client = client_for(store, test_params()).await;

    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(body.contains("bsky.app"), "home page should contain Bluesky links");
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

// ─── Auth tests ────────────────────────────────────────────────

async fn oauth_client(
    mock_server: &MockServer,
    allowed_users: Vec<&str>,
    dir: &tempfile::TempDir,
) -> Client {
    let store = open_temp_store(dir).await;
    let store = Arc::new(store);
    let params = test_params();
    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        params.max_entries,
        params.max_age_hours,
    ));
    let oauth_config = OAuthConfig::with_urls(
        "test-client-id".to_string(),
        "test-client-secret".to_string(),
        allowed_users.into_iter().map(String::from).collect(),
        format!("{}/authorize", mock_server.uri()),
        format!("{}/token", mock_server.uri()),
        mock_server.uri().to_string(),
    );
    let project = FeedProject {
        cache_layer: FeedCacheLayer::new(cache),
        feed_config_layer: FeedConfigLayer::new(params),
        store_layer: StoreLayer::from_shared(store),
        appraiser_layer: AppraiserLayer::new(None),
        oauth_config_layer: OAuthConfigLayer::new(Some(Arc::new(oauth_config))),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: None,
    };
    Client::new(project).await
}

/// Extract the session cookie value from a response's Set-Cookie header.
fn extract_session_cookie(response: &cot::http::Response<cot::Body>) -> Option<String> {
    response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").to_string())
}

/// Build a GET request with an optional session cookie.
fn get_with_cookie(uri: &str, cookie: Option<&str>) -> cot::http::Request<cot::Body> {
    get_with_cookie_and_headers(uri, cookie, &[])
}

/// Build a GET request with an optional session cookie and extra headers.
fn get_with_cookie_and_headers(
    uri: &str,
    cookie: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> cot::http::Request<cot::Body> {
    let mut builder = cot::http::Request::builder().uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    builder.body(cot::Body::empty()).expect("build request")
}

/// Extract a query parameter from a URL string.
fn extract_query_param(url: &str, param: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if parts.next() == Some(param) {
            return parts
                .next()
                .map(|v| urlencoding::decode(v).unwrap_or_default().into_owned());
        }
    }
    None
}

/// Mount mock responses for GitHub token exchange and /user API.
async fn mount_github_mocks(mock_server: &MockServer, github_username: &str) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "access_token": "test-access-token",
                    "token_type": "bearer"
                })),
        )
        .mount(mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "login": github_username,
                })),
        )
        .mount(mock_server)
        .await;
}

/// Perform a full login flow: /auth/login → capture state → /auth/callback.
/// Returns the session cookie and final response.
async fn do_login(
    client: &mut Client,
    cookie: Option<&str>,
) -> (cot::http::Response<cot::Body>, String) {
    // Step 1: GET /auth/login to get CSRF state and session cookie
    let response = client
        .request(get_with_cookie("/auth/login", cookie))
        .await
        .expect("GET /auth/login");
    assert_eq!(response.status().as_u16(), 303, "login should redirect");
    let location = response
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header")
        .to_string();
    let session_cookie = extract_session_cookie(&response).expect("session cookie set");
    let state = extract_query_param(&location, "state").expect("state param in redirect URL");

    // Step 2: GET /auth/callback with CSRF state and session cookie
    let callback_uri = format!("/auth/callback?code=test-code&state={state}");
    let response = client
        .request(get_with_cookie(&callback_uri, Some(&session_cookie)))
        .await
        .expect("GET /auth/callback");

    // Update cookie if a new one was set
    let callback_cookie = extract_session_cookie(&response);
    let final_cookie = callback_cookie.unwrap_or(session_cookie);
    (response, final_cookie)
}

#[tokio::test]
async fn unauthenticated_admin_redirects_to_login() {
    let mock_server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut client = oauth_client(&mock_server, vec!["testuser"], &dir).await;

    let response = client
        .request(get_with_cookie("/admin", None))
        .await
        .expect("GET /admin");
    assert_eq!(response.status().as_u16(), 303);
    let location = response
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header");
    assert!(
        location.starts_with("/auth/login"),
        "should redirect to /auth/login, got: {location}"
    );
    assert!(
        location.contains("return_to"),
        "should include return_to param"
    );
}

#[tokio::test]
async fn allowlisted_user_lands_on_admin_after_login() {
    let mock_server = MockServer::start().await;
    mount_github_mocks(&mock_server, "testuser").await;
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut client = oauth_client(&mock_server, vec!["testuser"], &dir).await;

    let (response, cookie) = do_login(&mut client, None).await;
    assert_eq!(
        response.status().as_u16(),
        303,
        "callback should redirect after successful login"
    );
    let location = response
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header");
    assert_eq!(location, "/admin", "should redirect to /admin");

    // Subsequent GET /admin should succeed (not redirect to login)
    let response = client
        .request(get_with_cookie("/admin", Some(&cookie)))
        .await
        .expect("GET /admin after login");
    assert_eq!(
        response.status().as_u16(),
        200,
        "authenticated admin request should return 200"
    );
}

#[tokio::test]
async fn non_allowlisted_user_gets_403() {
    let mock_server = MockServer::start().await;
    mount_github_mocks(&mock_server, "eviluser").await;
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut client = oauth_client(&mock_server, vec!["testuser"], &dir).await;

    let (response, _) = do_login(&mut client, None).await;
    assert_eq!(
        response.status().as_u16(),
        403,
        "non-allowlisted user should get 403"
    );
    let body_bytes = response
        .into_body()
        .into_bytes()
        .await
        .expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("valid utf8");
    assert!(
        body.contains("eviluser"),
        "403 body should mention the rejected username"
    );
}

#[tokio::test]
async fn logout_clears_session_and_admin_redirects_again() {
    let mock_server = MockServer::start().await;
    mount_github_mocks(&mock_server, "testuser").await;
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut client = oauth_client(&mock_server, vec!["testuser"], &dir).await;

    // Login
    let (_, cookie) = do_login(&mut client, None).await;

    // Verify admin works
    let response = client
        .request(get_with_cookie("/admin", Some(&cookie)))
        .await
        .expect("GET /admin");
    assert_eq!(response.status().as_u16(), 200);

    // Logout
    let response = client
        .request(get_with_cookie("/auth/logout", Some(&cookie)))
        .await
        .expect("GET /auth/logout");
    assert_eq!(response.status().as_u16(), 303);
    let location = response
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header");
    assert_eq!(location, "/", "logout should redirect to /");
    let post_logout_cookie = extract_session_cookie(&response).unwrap_or(cookie);

    // Admin should redirect to login again
    let response = client
        .request(get_with_cookie("/admin", Some(&post_logout_cookie)))
        .await
        .expect("GET /admin after logout");
    assert_eq!(
        response.status().as_u16(),
        303,
        "admin should redirect to login after logout"
    );
}

#[tokio::test]
async fn tampered_csrf_state_is_rejected() {
    let mock_server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut client = oauth_client(&mock_server, vec!["testuser"], &dir).await;

    // Start login to get a session cookie
    let response = client
        .request(get_with_cookie("/auth/login", None))
        .await
        .expect("GET /auth/login");
    let cookie = extract_session_cookie(&response).expect("session cookie");

    // Call callback with tampered state
    let response = client
        .request(get_with_cookie(
            "/auth/callback?code=test-code&state=tampered-state",
            Some(&cookie),
        ))
        .await
        .expect("GET /auth/callback with tampered state");
    assert_eq!(
        response.status().as_u16(),
        403,
        "tampered CSRF state should be rejected with 403"
    );
    let body_bytes = response
        .into_body()
        .into_bytes()
        .await
        .expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("valid utf8");
    assert!(
        body.contains("CSRF"),
        "rejection should mention CSRF: {body}"
    );
}

#[tokio::test]
async fn login_redirect_uses_https_when_x_forwarded_proto_is_set() {
    let mock_server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut client = oauth_client(&mock_server, vec!["testuser"], &dir).await;

    let request = get_with_cookie_and_headers(
        "/auth/login",
        None,
        &[("x-forwarded-proto", "https")],
    );
    let response = client.request(request).await.expect("GET /auth/login");
    assert_eq!(response.status().as_u16(), 303, "login should redirect");

    let location = response
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header");
    let redirect_uri =
        extract_query_param(location, "redirect_uri").expect("redirect_uri in OAuth URL");
    assert!(
        redirect_uri.starts_with("https://"),
        "redirect_uri should use https when x-forwarded-proto is https, got: {redirect_uri}"
    );
}
