#![cfg(feature = "test")]

mod common;

use std::sync::Arc;

use chrono::Utc;
use common::{
    extract_query_param, extract_session_cookie, get_with_cookie, get_with_cookie_and_headers,
    mount_github_mocks,
};
use std::time::Duration;

use bluesky::ImageUrl;
use cot::test::Client;
use deadpool_redis::redis::aio::MultiplexedConnection;
use shared::{Appraiser, BlueskyCid, ImageId};
use skeet_appraise::auth_config::OAuthConfig;
use skeet_appraise::available_feeds::PublishedListCatalogReader;
use skeet_appraise::project::AppraiseProject;
use skeet_appraise::{
    AppraiserLayer, ModelsLayer, OAuthConfigLayer, PublishedFeedLayer, StartedAtLayer, StoreLayer,
};
use skeet_publish::{Limit, Order, PublishedImage, PublishedList, PublishedListCatalog, connect};
use skeet_store::test_utils::{make_record, make_record_at, open_temp_store, test_image};
use skeet_store::{
    DiscoveredAt, ImageRecord, ModelVersion, OriginalAt, Score, Scores, SkeetId, SkeetStore, Zone,
};
use test_support::test_models;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::time::sleep;
use wiremock::MockServer;

/// An unreachable url for the published-feed reader. The home page is the only
/// handler that reads it, so the other tests never connect.
const DUMMY_REDIS_URL: &str = "redis://127.0.0.1:1";

async fn client_for(store: SkeetStore) -> Client {
    client_for_with_feed(Arc::new(store), DUMMY_REDIS_URL).await
}

/// A feed-catalog reader against `redis_url`. The home page discovers its feeds
/// from the catalog there (seed it with [`seed_test_catalog`]); handlers other
/// than home never connect.
fn test_feeds(redis_url: &str) -> Arc<PublishedListCatalogReader> {
    Arc::new(PublishedListCatalogReader::new(redis_url))
}

/// Advertise the feeds the home page offers in tests via the catalog. None is
/// `quality-4w`, so the default falls back to the first in dropdown order
/// (`quality-48h`); `recency-7d` is absent, so requesting it is rejected.
async fn seed_test_catalog(conn: &mut MultiplexedConnection) {
    let lists = [
        PublishedList::new(Order::Quality, Limit::hours(48)),
        PublishedList::new(Order::Quality, Limit::days(7)),
        PublishedList::new(Order::Recency, Limit::hours(48)),
    ];
    PublishedListCatalog::write(conn, &lists)
        .await
        .expect("seed catalog");
}

async fn client_for_with_feed(store: Arc<SkeetStore>, redis_url: &str) -> Client {
    let project = AppraiseProject {
        published_feed_layer: PublishedFeedLayer::new(test_feeds(redis_url)),
        store_layer: StoreLayer::from_shared(store),
        models_layer: ModelsLayer::from_shared(test_models()),
        appraiser_layer: AppraiserLayer::new(Some(Arc::new(Appraiser::LocalAdmin))),
        oauth_config_layer: OAuthConfigLayer::new(None),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: None,
        use_redis: false,
        redis_url: None,
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

/// Add a record and score it, returning (skeet_id string, image_id string).
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

// ─── Static assets ──────────────────────────────────────────────

#[tokio::test]
async fn static_htmx_js_is_served() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let mut client = client_for(store).await;

    let (status, body) = get_body(&mut client, "/static/htmx.min.js").await;
    assert_eq!(
        status, 200,
        "htmx.min.js should be served at /static/htmx.min.js"
    );
    assert!(body.contains("htmx"), "response should contain htmx code");
}

// ─── Annotated image caching ────────────────────────────────────

#[tokio::test]
async fn annotated_image_returns_last_modified_header() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "img1", 10, 0.85).await;

    let mut client = client_for(store).await;

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

    let mut client = client_for(store).await;

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

// ─── Home page ─────────────────────────────────────────────────

/// A real Bluesky blob CID, for the V3 image id the home test joins on.
const HOME_CID: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

async fn connect_ready(url: &str) -> MultiplexedConnection {
    for _ in 0..100 {
        if let Ok(conn) = connect(url).await {
            return conn;
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("redis container not reachable at {url} within 10s");
}

/// Add and score a V3 store image for `cid`/`rkey`, returning a `PublishedImage`
/// referencing it (its `image_id` matches the store record, so the home join
/// finds the score).
async fn seed_and_publishable(
    store: &SkeetStore,
    rkey: &str,
    cid: &str,
    score: f32,
) -> PublishedImage {
    let image_id = ImageId::V3(BlueskyCid::new(cid).expect("valid cid"));
    let skeet_id: SkeetId = format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
        .parse()
        .expect("valid skeet id");
    let record = ImageRecord {
        image_id: image_id.clone(),
        skeet_id: skeet_id.clone(),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    };
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &image_id,
            &Score::new(score).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");
    PublishedImage::unprobed(
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{cid}@jpeg"
        ))
        .expect("valid url"),
        image_id,
        skeet_id,
    )
}

/// The home page renders exactly the published list joined to live store detail:
/// publish one item whose `image_id` matches a scored store image, then assert
/// the page shows it (Bluesky link + the joined-in score).
#[tokio::test]
async fn home_page_shows_published_entries_docker() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = Arc::new(open_temp_store(&dir).await);

    // A V3 scored image in the store.
    let image_id = ImageId::V3(BlueskyCid::new(HOME_CID).expect("valid cid"));
    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/home1"
        .parse()
        .expect("valid skeet id");
    let record = ImageRecord {
        image_id: image_id.clone(),
        skeet_id: skeet_id.clone(),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    };
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &image_id,
            &Score::new(0.85).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");

    // Publish a matching item to a testcontainers redis.
    let container = Redis::default().start().await.expect("start redis");
    let host = container.get_host().await.expect("redis host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("redis port");
    let redis_url = format!("redis://{host}:{port}");
    let mut conn = connect_ready(&redis_url).await;
    let published = PublishedImage::unprobed(
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{HOME_CID}@jpeg"
        ))
        .expect("valid url"),
        image_id,
        skeet_id,
    );
    PublishedList::new(Order::Quality, Limit::hours(48))
        .replace(&mut conn, &[published], Utc::now())
        .await
        .expect("publish");
    seed_test_catalog(&mut conn).await;

    let mut client = client_for_with_feed(Arc::clone(&store), &redis_url).await;
    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(
        body.contains("bsky.app/profile/did:plc:abc/post/home1"),
        "home should link the published skeet"
    );
    assert!(
        body.contains("0.85"),
        "home should show the joined-in score"
    );
}

/// The home page reads the published list named by `?feed=`, defaulting to
/// `quality-48h`. Publish distinct items to `quality-48h` and `quality-7d` and
/// assert each selection shows its own item, with the dropdown defaulting to
/// `quality-48h`.
#[tokio::test]
async fn home_selects_feed_from_query_docker() {
    const CID_48H: &str = "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CID_7D: &str = "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    let dir = tempfile::tempdir().expect("create temp dir");
    let store = Arc::new(open_temp_store(&dir).await);

    let item_48h = seed_and_publishable(&store, "q48", CID_48H, 0.80).await;
    let item_7d = seed_and_publishable(&store, "q7d", CID_7D, 0.80).await;

    let container = Redis::default().start().await.expect("start redis");
    let host = container.get_host().await.expect("redis host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("redis port");
    let redis_url = format!("redis://{host}:{port}");
    let mut conn = connect_ready(&redis_url).await;
    PublishedList::new(Order::Quality, Limit::hours(48))
        .replace(&mut conn, &[item_48h], Utc::now())
        .await
        .expect("publish 48h");
    PublishedList::new(Order::Quality, Limit::days(7))
        .replace(&mut conn, &[item_7d], Utc::now())
        .await
        .expect("publish 7d");
    seed_test_catalog(&mut conn).await;

    let mut client = client_for_with_feed(Arc::clone(&store), &redis_url).await;

    // Default selection is quality-48h: shows its item, marks the dropdown
    // default, and does not show the quality-7d item.
    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(
        body.contains("post/q48"),
        "default should show the quality-48h item"
    );
    assert!(
        !body.contains("post/q7d"),
        "default should not show the quality-7d item"
    );
    assert!(
        body.contains("post/q48"),
        "default should show the quality-48h item"
    );
    assert!(
        !body.contains("post/q7d"),
        "default should not show the quality-7d item"
    );
    assert!(
        body.contains(r#"value="quality-48h" selected"#),
        "dropdown should default to quality-48h"
    );

    // Selecting quality-7d shows that list's item instead.
    let (status, body) = get_body(&mut client, "/?feed=quality-7d").await;
    assert_eq!(status, 200);
    assert!(
        body.contains("post/q7d"),
        "?feed=quality-7d should show the 7d item"
    );
    assert!(
        !body.contains("post/q48"),
        "?feed=quality-7d should not show the 48h item"
    );
    assert!(
        body.contains("post/q7d"),
        "?feed=quality-7d should show the 7d item"
    );
    assert!(
        !body.contains("post/q48"),
        "?feed=quality-7d should not show the 48h item"
    );
    assert!(
        body.contains(r#"value="quality-7d" selected"#),
        "dropdown should mark quality-7d selected"
    );

    // An explicit unknown feed is rejected, not silently defaulted.
    let (status, _) = get_body(&mut client, "/?feed=recency-7d").await;
    assert_eq!(status, 400, "an unconfigured feed should be a bad request");
}

/// A manual skeet band must cap the band shown on the home view: a `0.95` score
/// bands `HighQuality`, but a manual skeet band of `MediumHigh` should pull the
/// displayed (feed-effective) band down to `MediumHigh` — the value the
/// feed/quality sort publishes — not leave the image's higher band showing.
#[tokio::test]
async fn home_effective_band_is_capped_by_manual_skeet_band_docker() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = Arc::new(open_temp_store(&dir).await);

    // A V3 image scored 0.95 (→ HighQuality under the t=0.5 test model).
    let image_id = ImageId::V3(BlueskyCid::new(HOME_CID).expect("valid cid"));
    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/cap1"
        .parse()
        .expect("valid skeet id");
    let record = ImageRecord {
        image_id: image_id.clone(),
        skeet_id: skeet_id.clone(),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    };
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &image_id,
            &Score::new(0.95).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");

    // Publish the matching item to a testcontainers redis.
    let container = Redis::default().start().await.expect("start redis");
    let host = container.get_host().await.expect("redis host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("redis port");
    let redis_url = format!("redis://{host}:{port}");
    let mut conn = connect_ready(&redis_url).await;
    let published = PublishedImage::unprobed(
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{HOME_CID}@jpeg"
        ))
        .expect("valid url"),
        image_id,
        skeet_id.clone(),
    );
    PublishedList::new(Order::Quality, Limit::hours(48))
        .replace(&mut conn, &[published], Utc::now())
        .await
        .expect("publish");
    seed_test_catalog(&mut conn).await;

    let mut client = client_for_with_feed(Arc::clone(&store), &redis_url).await;

    // Manually band the skeet MediumHigh via the public appraise endpoint.
    appraise(&mut client, "skeet", &skeet_id.to_string(), "MediumHigh").await;

    let (status, body) = get_body(&mut client, "/").await;
    assert_eq!(status, 200);
    assert!(
        body.contains(r#"<span class="band MediumHigh">MediumHigh</span>"#),
        "home should show the capped effective band MediumHigh"
    );
    assert!(
        !body.contains(r#"<span class="band HighQuality">HighQuality</span>"#),
        "home should not show the uncapped image band HighQuality"
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

    let mut client = client_for(store).await;

    // First page: 10 newest items (item0..item9)
    let (status, body) = get_body(&mut client, "/admin").await;
    assert_eq!(status, 200);
    let first_page_ids = extract_item_ids(&body);
    assert_eq!(first_page_ids.len(), 10);
    assert_eq!(first_page_ids, skeet_ids[..10]);

    // Extract cursor from the htmx load-more div
    let cursor_marker = r#"hx-get="/admin?view=skeet&cursor="#;
    let cursor_pos = body
        .find(cursor_marker)
        .expect("should have a next-page cursor");
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

    let mut client = client_for(store).await;

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

    let mut client = client_for(store).await;

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

    let mut client = client_for(store).await;

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

    let mut client = client_for(store).await;

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

/// The effective-band cell is the last `band-tag` before the appraise buttons
/// (columns run: auto, manual, effective, then the buttons).
fn extract_effective_band(row_html: &str) -> Option<String> {
    let buttons_pos = row_html.find("band-buttons")?;
    let before = &row_html[..buttons_pos];
    let tag = r#"<span class="band-tag "#;
    let last = before.rfind(tag)?;
    let after = &before[last + tag.len()..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

/// In the image view, a manual *skeet* band must cap a higher image band: a `0.95`
/// score bands `HighQuality`, but a manual skeet band of `MediumHigh` pulls the
/// effective band down to `MediumHigh` — the feed-effective value.
#[tokio::test]
async fn admin_image_view_effective_band_capped_by_manual_skeet_band() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "capimg", 10, 0.95).await;

    let mut client = client_for(store).await;
    appraise(&mut client, "skeet", &skeet_id, "MediumHigh").await;

    let (status, body) = get_body(&mut client, "/admin?view=image").await;
    assert_eq!(status, 200);
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert_eq!(rows_html.len(), 1);
    assert_eq!(
        extract_effective_band(rows_html[0]).as_deref(),
        Some("MediumHigh"),
        "image-view effective band should be capped to MediumHigh by the manual skeet band"
    );
}

/// In the skeet view, a higher manual skeet band must still be capped by a lower
/// image band: a `0.6` score bands `MediumHigh`, so a manual skeet band of
/// `HighQuality` cannot raise the effective band above `MediumHigh`.
#[tokio::test]
async fn admin_skeet_view_effective_band_capped_by_lower_image_band() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "capskeet", 10, 0.6).await;

    let mut client = client_for(store).await;
    appraise(&mut client, "skeet", &skeet_id, "HighQuality").await;

    let (status, body) = get_body(&mut client, "/admin?view=skeet").await;
    assert_eq!(status, 200);
    let rows_html: Vec<&str> = body.split("<tr id=").skip(1).collect();
    assert_eq!(rows_html.len(), 1);
    assert_eq!(
        extract_effective_band(rows_html[0]).as_deref(),
        Some("MediumHigh"),
        "skeet-view effective band should be capped to MediumHigh by the lower image band"
    );
}

#[tokio::test]
async fn appraise_skeet_returns_row_html() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (skeet_id, _) = seed_scored(&store, "approw1", 10, 0.85).await;

    let mut client = client_for(store).await;

    let encoded_id = urlencoding::encode(&skeet_id);
    let (status, body) = get_body(
        &mut client,
        &format!("/admin/appraise/skeet?band=Low&id={encoded_id}"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        body.contains("<tr"),
        "appraise response should contain a table row"
    );
    assert!(
        body.contains("Low"),
        "appraise response should show the new band"
    );
}

#[tokio::test]
async fn appraise_image_returns_row_html() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    let (_, image_id) = seed_scored(&store, "appimg1", 10, 0.85).await;

    let mut client = client_for(store).await;

    let encoded_id = urlencoding::encode(&image_id);
    let (status, body) = get_body(
        &mut client,
        &format!("/admin/appraise/image?band=HighQuality&id={encoded_id}"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(
        body.contains("<tr"),
        "appraise response should contain a table row"
    );
    assert!(
        body.contains("HighQuality"),
        "appraise response should show the new band"
    );
}

// ─── Auth tests ────────────────────────────────────────────────

async fn oauth_client(
    mock_server: &MockServer,
    allowed_users: Vec<&str>,
    dir: &tempfile::TempDir,
) -> Client {
    let store = Arc::new(open_temp_store(dir).await);
    let oauth_config = OAuthConfig::with_urls(
        "test-client-id".to_string(),
        "test-client-secret".to_string(),
        allowed_users.into_iter().map(String::from).collect(),
        format!("{}/authorize", mock_server.uri()),
        format!("{}/token", mock_server.uri()),
        mock_server.uri().to_string(),
    );
    let project = AppraiseProject {
        published_feed_layer: PublishedFeedLayer::new(test_feeds(DUMMY_REDIS_URL)),
        store_layer: StoreLayer::from_shared(store),
        models_layer: ModelsLayer::from_shared(test_models()),
        appraiser_layer: AppraiserLayer::new(None),
        oauth_config_layer: OAuthConfigLayer::new(Some(Arc::new(oauth_config))),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: None,
        use_redis: false,
        redis_url: None,
    };
    Client::new(project).await
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
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
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
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
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

    let request =
        get_with_cookie_and_headers("/auth/login", None, &[("x-forwarded-proto", "https")]);
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
