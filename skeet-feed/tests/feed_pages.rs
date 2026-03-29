#![cfg(feature = "test")]

use chrono::Utc;
use cot::test::Client;
use image::{DynamicImage, ImageBuffer, Rgba};
use skeet_feed::StoreLayer;
use skeet_feed::project::FeedProject;
use skeet_store::{
    DiscoveredAt, ImageId, ImageRecord, ModelVersion, OriginalAt, Score, SkeetStore, Zone,
};

fn test_image() -> DynamicImage {
    DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255])))
}

fn make_record(suffix: &str, r: u8, g: u8, b: u8) -> ImageRecord {
    let img = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([r, g, b, 255])));
    ImageRecord {
        image_id: ImageId::from_image(&img),
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{suffix}")
            .parse()
            .expect("valid AT URI"),
        image: img,
        discovered_at: DiscoveredAt::now(),
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

async fn client_for(store: SkeetStore) -> Client {
    let project = FeedProject {
        store_layer: StoreLayer::new(store),
    };
    Client::new(project).await
}

async fn get_body(client: &mut Client, path: &str) -> String {
    let response = client.get(path).await.expect("GET request");
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
    String::from_utf8(body_bytes.to_vec()).expect("valid utf8")
}

fn has_table_rows(body: &str) -> bool {
    body.contains("<tr>")
}

#[tokio::test]
async fn latest_shows_entries_when_store_has_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("latest1", 0, 255, 0);
    store.add(&record).await.expect("add record");

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/latest").await;

    assert!(has_table_rows(&body), "expected table rows in latest feed");
}

#[tokio::test]
async fn latest_shows_no_entries_when_store_has_no_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/latest").await;

    assert!(
        !has_table_rows(&body),
        "expected no table rows in empty latest feed"
    );
}

#[tokio::test]
async fn best_shows_entries_when_store_has_scored_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("best1", 0, 0, 255);
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

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/best").await;

    assert!(has_table_rows(&body), "expected table rows in best feed");
}

#[tokio::test]
async fn best_shows_no_entries_when_store_has_no_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/best").await;

    assert!(
        !has_table_rows(&body),
        "expected no table rows in empty best feed"
    );
}

#[tokio::test]
async fn best_shows_no_entries_when_store_has_skeets_but_none_scored() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("unscored1", 128, 128, 0);
    store.add(&record).await.expect("add record");

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/best").await;

    assert!(
        !has_table_rows(&body),
        "expected no table rows in best feed when no skeets are scored"
    );
}
