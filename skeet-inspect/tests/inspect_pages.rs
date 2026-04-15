#![cfg(feature = "test")]

use cot::test::Client;
use skeet_inspect::project::InspectProject;
use skeet_store::test_utils::{make_record, open_temp_store};
use skeet_store::{ModelVersion, Score, SkeetStore};
use skeet_web_shared::StoreLayer;

async fn client_for(store: SkeetStore) -> Client {
    let project = InspectProject {
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
async fn pruned_shows_entries_when_store_has_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("pruned1", 0, 255, 0);
    store.add(&record).await.expect("add record");

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/pruned").await;

    assert!(has_table_rows(&body), "expected table rows in pruned page");
}

#[tokio::test]
async fn pruned_shows_no_entries_when_store_has_no_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/pruned").await;

    assert!(
        !has_table_rows(&body),
        "expected no table rows in empty pruned page"
    );
}

#[tokio::test]
async fn refined_shows_entries_when_store_has_scored_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("refined1", 0, 0, 255);
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
    let body = get_body(&mut client, "/refined").await;

    assert!(
        has_table_rows(&body),
        "expected table rows in refined page"
    );
}

#[tokio::test]
async fn refined_shows_no_entries_when_store_has_no_skeets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/refined").await;

    assert!(
        !has_table_rows(&body),
        "expected no table rows in empty refined page"
    );
}

#[tokio::test]
async fn refined_shows_no_entries_when_store_has_skeets_but_none_scored() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;

    let record = make_record("unscored1", 128, 128, 0);
    store.add(&record).await.expect("add record");

    let mut client = client_for(store).await;
    let body = get_body(&mut client, "/refined").await;

    assert!(
        !has_table_rows(&body),
        "expected no table rows in refined page when no skeets are scored"
    );
}
