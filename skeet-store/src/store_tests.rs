use chrono::Utc;

use crate::test_utils::{open_temp_store, test_image, test_image_with_color};
use crate::{
    Appraisal, Appraiser, Band, DiscoveredAt, ImageId, ImageRecord, ModelVersion, OriginalAt,
    Score, SkeetId, SkeetStore, Zone,
};

#[tokio::test]
async fn roundtrip_store_and_retrieve() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    assert_eq!(store.count().await.unwrap(), 0);

    let record = ImageRecord {
        image_id: ImageId::from_image(&test_image()),
        skeet_id: "at://did:plc:abc/app.bsky.feed.post/123"
            .parse()
            .expect("valid test AT URI"),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    };

    store.add(&record).await.unwrap();
    assert_eq!(store.count().await.unwrap(), 1);

    let images = store.list_all().await.unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].summary.image_id, record.image_id);
    assert_eq!(images[0].summary.skeet_id, record.skeet_id);
    assert_eq!(images[0].image.width(), 2);
    assert_eq!(images[0].image.height(), 2);
    assert_eq!(images[0].summary.zone, Zone::TopRight);
}

#[tokio::test]
async fn multiple_images_per_skeet() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/456"
        .parse()
        .expect("valid test AT URI");

    for i in 0..3 {
        let img = test_image_with_color(i * 80, 0, 0);
        let record = ImageRecord {
            image_id: ImageId::from_image(&img),
            skeet_id: skeet_id.clone(),
            image: img,
            discovered_at: DiscoveredAt::now(),
            original_at: OriginalAt::new(Utc::now()),
            zone: Zone::BottomLeft,
            annotated_image: test_image(),
            config_version: ModelVersion::from("test"),
            detected_text: String::new(),
        };
        store.add(&record).await.unwrap();
    }

    assert_eq!(store.count().await.unwrap(), 3);

    let unique_skeets = store.unique_skeet_ids().await.unwrap();
    assert_eq!(unique_skeets.len(), 1);
    assert_eq!(unique_skeets[0], skeet_id);
}

#[tokio::test]
async fn list_all_summaries() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let record = ImageRecord {
        image_id: ImageId::from_image(&test_image()),
        skeet_id: "at://did:plc:abc/app.bsky.feed.post/summ"
            .parse()
            .expect("valid test AT URI"),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::BottomRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    };

    store.add(&record).await.unwrap();

    let summaries = store.list_all_summaries().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].image_id, record.image_id);
    assert_eq!(summaries[0].skeet_id, record.skeet_id);
    assert_eq!(summaries[0].zone, Zone::BottomRight);
}

#[tokio::test]
async fn reopening_store_preserves_data() {
    let dir = tempfile::tempdir().unwrap();

    let record = ImageRecord {
        image_id: ImageId::from_image(&test_image()),
        skeet_id: "at://did:plc:abc/app.bsky.feed.post/789"
            .parse()
            .expect("valid test AT URI"),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopLeft,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    };

    {
        let store = open_temp_store(&dir).await;
        store.add(&record).await.unwrap();
    }

    let store = open_temp_store(&dir).await;
    assert_eq!(store.count().await.unwrap(), 1);
}

fn make_record(skeet_suffix: &str) -> ImageRecord {
    crate::test_utils::make_record(skeet_suffix, rand::random(), rand::random(), rand::random())
}

fn test_model_version() -> ModelVersion {
    ModelVersion::from("test_v1")
}

#[tokio::test]
async fn upsert_and_read_score() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record = make_record("score1");
    store.add(&record).await.unwrap();

    assert_eq!(store.get_score(&record.image_id).await.unwrap(), None);

    let score = Score::new(0.75).expect("valid score");
    let mv = test_model_version();
    store
        .upsert_score(&record.image_id, &score, &mv)
        .await
        .unwrap();
    let result = store.get_score(&record.image_id).await.unwrap();
    assert_eq!(result, Some((score, mv)));
}

#[tokio::test]
async fn upsert_overwrites_existing_score() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record = make_record("score2");
    store.add(&record).await.unwrap();

    let mv = test_model_version();
    store
        .upsert_score(&record.image_id, &Score::new(0.5).expect("valid"), &mv)
        .await
        .unwrap();
    let new_score = Score::new(0.9).expect("valid");
    store
        .upsert_score(&record.image_id, &new_score, &mv)
        .await
        .unwrap();

    let result = store.get_score(&record.image_id).await.unwrap();
    assert_eq!(result, Some((new_score, mv)));
}

#[tokio::test]
async fn list_unscored_returns_images_without_scores() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let r1 = make_record("unscored1");
    let r2 = make_record("unscored2");
    let r3 = make_record("unscored3");
    store.add(&r1).await.unwrap();
    store.add(&r2).await.unwrap();
    store.add(&r3).await.unwrap();

    let mv = test_model_version();
    store
        .upsert_score(&r1.image_id, &Score::new(0.8).expect("valid"), &mv)
        .await
        .unwrap();

    let unscored = store
        .list_unscored_image_ids_for_version(&mv)
        .await
        .unwrap();
    assert_eq!(unscored.len(), 2);
    assert!(unscored.contains(&r2.image_id));
    assert!(unscored.contains(&r3.image_id));
    assert!(!unscored.contains(&r1.image_id));
}

#[tokio::test]
async fn list_unscored_includes_images_scored_with_different_version() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let r1 = make_record("ver1");
    store.add(&r1).await.unwrap();

    let old_mv = ModelVersion::from("old_v1");
    let new_mv = ModelVersion::from("new_v2");
    store
        .upsert_score(&r1.image_id, &Score::new(0.8).expect("valid"), &old_mv)
        .await
        .unwrap();

    let unscored = store
        .list_unscored_image_ids_for_version(&new_mv)
        .await
        .unwrap();
    assert_eq!(unscored.len(), 1);
    assert!(unscored.contains(&r1.image_id));
}

#[tokio::test]
async fn writes_from_one_store_visible_to_another() {
    let dir = tempfile::tempdir().unwrap();
    let store1 = open_temp_store(&dir).await;
    let store2 = open_temp_store(&dir).await;

    let record = make_record("cross-store-visibility");
    store1.add(&record).await.unwrap();

    // store2 should see the record written by store1
    assert!(
        store2.exists(&record.image_id).await.unwrap(),
        "store2 should see record written by store1"
    );
    assert_eq!(store2.count().await.unwrap(), 1);
}

#[tokio::test]
async fn list_scored_summaries_ordered_by_score() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let r1 = make_record("scored1");
    let r2 = make_record("scored2");
    let r3 = make_record("scored3");
    store.add(&r1).await.unwrap();
    store.add(&r2).await.unwrap();
    store.add(&r3).await.unwrap();

    let mv = test_model_version();
    store
        .upsert_score(&r1.image_id, &Score::new(0.3).expect("valid"), &mv)
        .await
        .unwrap();
    store
        .upsert_score(&r2.image_id, &Score::new(0.9).expect("valid"), &mv)
        .await
        .unwrap();
    // r3 not scored

    let scored = store.list_scored_summaries_by_score(10, None).await.unwrap();
    assert_eq!(scored.len(), 2);
    assert_eq!(scored[0].0.image_id, r2.image_id);
    assert_eq!(scored[0].1, Score::new(0.9).expect("valid"));
    assert_eq!(scored[1].0.image_id, r1.image_id);
    assert_eq!(scored[1].1, Score::new(0.3).expect("valid"));
}

struct CacheTestFixture {
    store: SkeetStore,
    r1: ImageRecord,
    r2: ImageRecord,
    _dir: tempfile::TempDir,
}

async fn setup_cache_test(prefix: &str) -> CacheTestFixture {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let r1 = make_record(&format!("{prefix}1"));
    let r2 = make_record(&format!("{prefix}2"));
    store.add(&r1).await.unwrap();
    store.add(&r2).await.unwrap();

    let mv = test_model_version();
    store
        .upsert_score(&r1.image_id, &Score::new(0.5).expect("valid"), &mv)
        .await
        .unwrap();

    // Populate any internal cache
    let scored = store.list_scored_summaries_by_score(10, None).await.unwrap();
    assert_eq!(scored.len(), 1);
    assert_eq!(scored[0].0.image_id, r1.image_id);

    CacheTestFixture { store, r1, r2, _dir: dir }
}

async fn assert_scores_reflect_update(
    store: &SkeetStore,
    expected_first: &ImageId,
    expected_first_score: Score,
    expected_second: &ImageId,
    expected_second_score: Score,
) {
    let scored = store.list_scored_summaries_by_score(10, None).await.unwrap();
    assert_eq!(scored.len(), 2);
    assert_eq!(scored[0].0.image_id, *expected_first, "wrong image in first position");
    assert_eq!(scored[0].1, expected_first_score);
    assert_eq!(scored[1].0.image_id, *expected_second, "wrong image in second position");
    assert_eq!(scored[1].1, expected_second_score);
}

#[tokio::test]
async fn scores_cache_invalidated_after_write() {
    let f = setup_cache_test("cache").await;
    let mv = test_model_version();

    // Add a new score
    f.store
        .upsert_score(&f.r2.image_id, &Score::new(0.8).expect("valid"), &mv)
        .await
        .unwrap();
    assert_scores_reflect_update(
        &f.store,
        &f.r2.image_id, Score::new(0.8).expect("valid"),
        &f.r1.image_id, Score::new(0.5).expect("valid"),
    ).await;

    // Update an existing score
    f.store
        .upsert_score(&f.r1.image_id, &Score::new(0.95).expect("valid"), &mv)
        .await
        .unwrap();
    assert_scores_reflect_update(
        &f.store,
        &f.r1.image_id, Score::new(0.95).expect("valid"),
        &f.r2.image_id, Score::new(0.8).expect("valid"),
    ).await;
}

#[tokio::test]
async fn scores_cache_invalidated_after_batch_upsert() {
    let f = setup_cache_test("batch_cache").await;
    let mv = test_model_version();

    // Batch write: updates r1 and adds r2 in one call
    f.store
        .batch_upsert_scores(&[
            (f.r1.image_id.clone(), Score::new(0.6).expect("valid"), mv.clone()),
            (f.r2.image_id.clone(), Score::new(0.9).expect("valid"), mv.clone()),
        ])
        .await
        .unwrap();
    assert_scores_reflect_update(
        &f.store,
        &f.r2.image_id, Score::new(0.9).expect("valid"),
        &f.r1.image_id, Score::new(0.6).expect("valid"),
    ).await;
}

fn test_appraiser() -> Appraiser {
    Appraiser::new_github("testuser").expect("valid appraiser")
}

fn other_appraiser() -> Appraiser {
    Appraiser::new_github("otheruser").expect("valid appraiser")
}

#[tokio::test]
async fn skeet_band_set_get_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/appr1"
        .parse()
        .expect("valid");

    assert_eq!(store.get_skeet_band(&skeet_id).await.unwrap(), None);

    store
        .set_skeet_band(&skeet_id, Band::HighQuality, &test_appraiser())
        .await
        .unwrap();

    let appraisal = store.get_skeet_band(&skeet_id).await.unwrap().expect("should exist");
    assert_eq!(appraisal.band, Band::HighQuality);
    assert_eq!(appraisal.appraiser, test_appraiser());
}

#[tokio::test]
async fn skeet_band_set_overwrites_previous() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/appr2"
        .parse()
        .expect("valid");

    store
        .set_skeet_band(&skeet_id, Band::Low, &test_appraiser())
        .await
        .unwrap();
    store
        .set_skeet_band(&skeet_id, Band::MediumHigh, &other_appraiser())
        .await
        .unwrap();

    let appraisal = store.get_skeet_band(&skeet_id).await.unwrap().expect("should exist");
    assert_eq!(appraisal.band, Band::MediumHigh);
    assert_eq!(appraisal.appraiser, other_appraiser());
}

#[tokio::test]
async fn skeet_band_clear_removes_appraisal() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/appr3"
        .parse()
        .expect("valid");

    store
        .set_skeet_band(&skeet_id, Band::Low, &test_appraiser())
        .await
        .unwrap();
    store.clear_skeet_band(&skeet_id).await.unwrap();

    assert_eq!(store.get_skeet_band(&skeet_id).await.unwrap(), None);
}

#[tokio::test]
async fn list_all_skeet_appraisals_returns_all() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let id1: SkeetId = "at://did:plc:abc/app.bsky.feed.post/list1"
        .parse()
        .expect("valid");
    let id2: SkeetId = "at://did:plc:abc/app.bsky.feed.post/list2"
        .parse()
        .expect("valid");

    store.set_skeet_band(&id1, Band::Low, &test_appraiser()).await.unwrap();
    store.set_skeet_band(&id2, Band::HighQuality, &other_appraiser()).await.unwrap();

    let all = store.list_all_skeet_appraisals().await.unwrap();
    assert_eq!(all.len(), 2);

    let by_id: std::collections::HashMap<_, _> = all.into_iter().collect();
    assert_eq!(by_id[&id1], Appraisal { band: Band::Low, appraiser: test_appraiser() });
    assert_eq!(by_id[&id2], Appraisal { band: Band::HighQuality, appraiser: other_appraiser() });
}

#[tokio::test]
async fn image_band_set_get_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let record = make_record("img_appr1");
    store.add(&record).await.unwrap();

    assert_eq!(store.get_image_band(&record.image_id).await.unwrap(), None);

    store
        .set_image_band(&record.image_id, Band::MediumLow, &test_appraiser())
        .await
        .unwrap();

    let appraisal = store.get_image_band(&record.image_id).await.unwrap().expect("should exist");
    assert_eq!(appraisal.band, Band::MediumLow);
    assert_eq!(appraisal.appraiser, test_appraiser());
}

#[tokio::test]
async fn image_band_set_overwrites_previous() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let record = make_record("img_appr2");
    store.add(&record).await.unwrap();

    store
        .set_image_band(&record.image_id, Band::Low, &test_appraiser())
        .await
        .unwrap();
    store
        .set_image_band(&record.image_id, Band::HighQuality, &other_appraiser())
        .await
        .unwrap();

    let appraisal = store.get_image_band(&record.image_id).await.unwrap().expect("should exist");
    assert_eq!(appraisal.band, Band::HighQuality);
    assert_eq!(appraisal.appraiser, other_appraiser());
}

#[tokio::test]
async fn image_band_clear_removes_appraisal() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let record = make_record("img_appr3");
    store.add(&record).await.unwrap();

    store
        .set_image_band(&record.image_id, Band::MediumHigh, &test_appraiser())
        .await
        .unwrap();
    store.clear_image_band(&record.image_id).await.unwrap();

    assert_eq!(store.get_image_band(&record.image_id).await.unwrap(), None);
}

#[tokio::test]
async fn list_all_image_appraisals_returns_all() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let r1 = make_record("img_list1");
    let r2 = make_record("img_list2");
    store.add(&r1).await.unwrap();
    store.add(&r2).await.unwrap();

    store.set_image_band(&r1.image_id, Band::MediumLow, &test_appraiser()).await.unwrap();
    store.set_image_band(&r2.image_id, Band::HighQuality, &other_appraiser()).await.unwrap();

    let all = store.list_all_image_appraisals().await.unwrap();
    assert_eq!(all.len(), 2);

    let by_id: std::collections::HashMap<_, _> = all.into_iter().collect();
    assert_eq!(by_id[&r1.image_id], Appraisal { band: Band::MediumLow, appraiser: test_appraiser() });
    assert_eq!(by_id[&r2.image_id], Appraisal { band: Band::HighQuality, appraiser: other_appraiser() });
}

#[tokio::test]
async fn clear_nonexistent_appraisal_is_ok() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let skeet_id: SkeetId = "at://did:plc:abc/app.bsky.feed.post/noop"
        .parse()
        .expect("valid");
    store.clear_skeet_band(&skeet_id).await.unwrap();

    let record = make_record("noop_img");
    store.add(&record).await.unwrap();
    store.clear_image_band(&record.image_id).await.unwrap();
}
