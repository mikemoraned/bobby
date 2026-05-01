use chrono::Utc;

use crate::test_utils::{make_record_at, open_temp_store, test_image, test_image_with_color};
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

    let stored = store.get_by_id(&record.image_id).await.unwrap().unwrap();
    assert_eq!(stored.summary.image_id, record.image_id);
    assert_eq!(stored.summary.skeet_id, record.skeet_id);
    assert_eq!(stored.image.width(), 2);
    assert_eq!(stored.image.height(), 2);
    assert_eq!(stored.summary.zone, Zone::TopRight);
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
        .list_unscored_image_ids_for_version(&mv, None)
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
        .list_unscored_image_ids_for_version(&new_mv, None)
        .await
        .unwrap();
    assert_eq!(unscored.len(), 1);
    assert!(unscored.contains(&r1.image_id));
}

#[tokio::test]
async fn list_unscored_with_since_filter_returns_only_newer() {
    use chrono::TimeZone as _;
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let t_old = chrono::Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
    let t_mid = chrono::Utc.with_ymd_and_hms(2026, 4, 15, 0, 0, 0).unwrap();
    let t_new = chrono::Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap();

    let r_old = make_record_at("old", 10, 0, 0, DiscoveredAt::new(t_old));
    let r_mid = make_record_at("mid", 20, 0, 0, DiscoveredAt::new(t_mid));
    let r_new = make_record_at("new", 30, 0, 0, DiscoveredAt::new(t_new));
    store.add(&r_old).await.unwrap();
    store.add(&r_mid).await.unwrap();
    store.add(&r_new).await.unwrap();

    let mv = test_model_version();
    let cutoff = DiscoveredAt::new(t_mid);
    let unscored = store
        .list_unscored_image_ids_for_version(&mv, Some(&cutoff))
        .await
        .unwrap();

    assert_eq!(unscored.len(), 2, "rows at or after cutoff");
    assert!(unscored.contains(&r_new.image_id));
    assert!(unscored.contains(&r_mid.image_id), "cutoff is inclusive");
    assert!(!unscored.contains(&r_old.image_id));
}

#[tokio::test]
async fn list_all_image_ids_with_since_filter_returns_only_newer() {
    use chrono::TimeZone as _;
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let t_old = chrono::Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
    let t_new = chrono::Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap();

    let r_old = make_record_at("all_old", 10, 0, 0, DiscoveredAt::new(t_old));
    let r_new = make_record_at("all_new", 30, 0, 0, DiscoveredAt::new(t_new));
    store.add(&r_old).await.unwrap();
    store.add(&r_new).await.unwrap();

    let cutoff = DiscoveredAt::new(t_new);
    let ids = store
        .list_all_image_ids_by_most_recent(Some(&cutoff))
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&r_new.image_id));
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

    let scored = store
        .list_scored_summaries_by_score(10, None)
        .await
        .unwrap();
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
    let scored = store
        .list_scored_summaries_by_score(10, None)
        .await
        .unwrap();
    assert_eq!(scored.len(), 1);
    assert_eq!(scored[0].0.image_id, r1.image_id);

    CacheTestFixture {
        store,
        r1,
        r2,
        _dir: dir,
    }
}

async fn assert_scores_reflect_update(
    store: &SkeetStore,
    expected_first: &ImageId,
    expected_first_score: Score,
    expected_second: &ImageId,
    expected_second_score: Score,
) {
    let scored = store
        .list_scored_summaries_by_score(10, None)
        .await
        .unwrap();
    assert_eq!(scored.len(), 2);
    assert_eq!(
        scored[0].0.image_id, *expected_first,
        "wrong image in first position"
    );
    assert_eq!(scored[0].1, expected_first_score);
    assert_eq!(
        scored[1].0.image_id, *expected_second,
        "wrong image in second position"
    );
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
        &f.r2.image_id,
        Score::new(0.8).expect("valid"),
        &f.r1.image_id,
        Score::new(0.5).expect("valid"),
    )
    .await;

    // Update an existing score
    f.store
        .upsert_score(&f.r1.image_id, &Score::new(0.95).expect("valid"), &mv)
        .await
        .unwrap();
    assert_scores_reflect_update(
        &f.store,
        &f.r1.image_id,
        Score::new(0.95).expect("valid"),
        &f.r2.image_id,
        Score::new(0.8).expect("valid"),
    )
    .await;
}

#[tokio::test]
async fn scores_cache_invalidated_after_batch_upsert() {
    let f = setup_cache_test("batch_cache").await;
    let mv = test_model_version();

    // Batch write: updates r1 and adds r2 in one call
    f.store
        .batch_upsert_scores(&[
            (
                f.r1.image_id.clone(),
                Score::new(0.6).expect("valid"),
                mv.clone(),
            ),
            (
                f.r2.image_id.clone(),
                Score::new(0.9).expect("valid"),
                mv.clone(),
            ),
        ])
        .await
        .unwrap();
    assert_scores_reflect_update(
        &f.store,
        &f.r2.image_id,
        Score::new(0.9).expect("valid"),
        &f.r1.image_id,
        Score::new(0.6).expect("valid"),
    )
    .await;
}

#[tokio::test]
async fn batch_upsert_scores_version_increments_by_one() {
    let f = setup_cache_test("batch_version").await;
    let mv = test_model_version();

    let version_before = f.store.scores_table.version().await.unwrap();
    f.store
        .batch_upsert_scores(&[
            (
                f.r1.image_id.clone(),
                Score::new(0.6).expect("valid"),
                mv.clone(),
            ),
            (
                f.r2.image_id.clone(),
                Score::new(0.9).expect("valid"),
                mv.clone(),
            ),
            (
                make_record("batch_version_extra").image_id.clone(),
                Score::new(0.3).expect("valid"),
                mv.clone(),
            ),
        ])
        .await
        .unwrap();
    let version_after = f.store.scores_table.version().await.unwrap();
    assert_eq!(version_after - version_before, 1, "batch_upsert_scores must produce exactly one commit regardless of batch size");
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

    let appraisal = store
        .get_skeet_band(&skeet_id)
        .await
        .unwrap()
        .expect("should exist");
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

    let appraisal = store
        .get_skeet_band(&skeet_id)
        .await
        .unwrap()
        .expect("should exist");
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

    store
        .set_skeet_band(&id1, Band::Low, &test_appraiser())
        .await
        .unwrap();
    store
        .set_skeet_band(&id2, Band::HighQuality, &other_appraiser())
        .await
        .unwrap();

    let all = store.list_all_skeet_appraisals().await.unwrap();
    assert_eq!(all.len(), 2);

    let by_id: std::collections::HashMap<_, _> = all.into_iter().collect();
    assert_eq!(
        by_id[&id1],
        Appraisal {
            band: Band::Low,
            appraiser: test_appraiser()
        }
    );
    assert_eq!(
        by_id[&id2],
        Appraisal {
            band: Band::HighQuality,
            appraiser: other_appraiser()
        }
    );
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

    let appraisal = store
        .get_image_band(&record.image_id)
        .await
        .unwrap()
        .expect("should exist");
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

    let appraisal = store
        .get_image_band(&record.image_id)
        .await
        .unwrap()
        .expect("should exist");
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

    store
        .set_image_band(&r1.image_id, Band::MediumLow, &test_appraiser())
        .await
        .unwrap();
    store
        .set_image_band(&r2.image_id, Band::HighQuality, &other_appraiser())
        .await
        .unwrap();

    let all = store.list_all_image_appraisals().await.unwrap();
    assert_eq!(all.len(), 2);

    let by_id: std::collections::HashMap<_, _> = all.into_iter().collect();
    assert_eq!(
        by_id[&r1.image_id],
        Appraisal {
            band: Band::MediumLow,
            appraiser: test_appraiser()
        }
    );
    assert_eq!(
        by_id[&r2.image_id],
        Appraisal {
            band: Band::HighQuality,
            appraiser: other_appraiser()
        }
    );
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

#[tokio::test]
async fn get_by_id_returns_stored_image() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record = make_record("getbyid1");
    store.add(&record).await.unwrap();

    let found = store.get_by_id(&record.image_id).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().summary.image_id, record.image_id);
}

#[tokio::test]
async fn get_by_id_returns_none_for_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let fake_id: ImageId = "00000000-0000-0000-0000-000000000000".parse().unwrap();
    assert!(store.get_by_id(&fake_id).await.unwrap().is_none());
}

#[tokio::test]
async fn get_originals_by_ids_returns_images() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record1 = make_record("orig1");
    let record2 = make_record("orig2");
    store.add(&record1).await.unwrap();
    store.add(&record2).await.unwrap();

    let ids = vec![record1.image_id.clone(), record2.image_id.clone()];
    let originals = store.get_originals_by_ids(&ids).await.unwrap();
    assert_eq!(originals.len(), 2);
    let original_ids: Vec<_> = originals.iter().map(|o| &o.summary.image_id).collect();
    assert!(original_ids.contains(&&record1.image_id));
    assert!(original_ids.contains(&&record2.image_id));
}

#[tokio::test]
async fn get_originals_by_ids_returns_empty_for_no_ids() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let originals = store.get_originals_by_ids(&[]).await.unwrap();
    assert!(originals.is_empty());
}

#[tokio::test]
async fn exists_returns_false_for_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let fake_id: ImageId = "00000000-0000-0000-0000-000000000000".parse().unwrap();
    assert!(!store.exists(&fake_id).await.unwrap());
}

#[tokio::test]
async fn delete_removes_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record = make_record("del1");
    store.add(&record).await.unwrap();
    assert!(store.exists(&record.image_id).await.unwrap());

    store.delete_by_id(&record.image_id).await.unwrap();
    assert!(!store.exists(&record.image_id).await.unwrap());
    assert_eq!(store.count().await.unwrap(), 0);
}

#[tokio::test]
async fn validate_succeeds_on_healthy_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    store.validate().await.unwrap();
}

#[tokio::test]
async fn list_scored_summaries_rejects_excessive_limit() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let result = store.list_scored_summaries_by_score(101, None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_scores_for_ids_returns_matching() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;

    let r1 = make_record("lsfi1");
    let r2 = make_record("lsfi2");
    store.add(&r1).await.unwrap();
    store.add(&r2).await.unwrap();

    let mv = test_model_version();
    let s1 = Score::new(0.7).expect("valid");
    store.upsert_score(&r1.image_id, &s1, &mv).await.unwrap();

    let id1_str = r1.image_id.to_string();
    let id2_str = r2.image_id.to_string();
    let scores = store
        .list_scores_for_ids(&[&id1_str, &id2_str])
        .await
        .unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[&r1.image_id], s1);
}

#[tokio::test]
async fn list_scored_summaries_filters_by_max_age() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let mv = test_model_version();

    // Old record (3 days ago)
    let old = make_record_at(
        "age_old",
        50,
        0,
        0,
        DiscoveredAt::new(Utc::now() - chrono::Duration::hours(72)),
    );
    store.add(&old).await.unwrap();
    store
        .upsert_score(&old.image_id, &Score::new(0.9).expect("valid"), &mv)
        .await
        .unwrap();

    // Recent record
    let recent = make_record("age_recent");
    store.add(&recent).await.unwrap();
    store
        .upsert_score(&recent.image_id, &Score::new(0.5).expect("valid"), &mv)
        .await
        .unwrap();

    // With 24h max age, only recent should appear
    let scored = store
        .list_scored_summaries_by_score(10, Some(24))
        .await
        .unwrap();
    assert_eq!(scored.len(), 1);
    assert_eq!(scored[0].0.image_id, recent.image_id);

    // With None (no age filter), both should appear
    let scored_all = store
        .list_scored_summaries_by_score(10, None)
        .await
        .unwrap();
    assert_eq!(scored_all.len(), 2);
}

#[tokio::test]
async fn compact_succeeds_on_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    store.compact().await.unwrap();
}

#[tokio::test]
async fn compact_preserves_data() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record = make_record("compact1");
    store.add(&record).await.unwrap();

    let mv = test_model_version();
    store
        .upsert_score(&record.image_id, &Score::new(0.8).expect("valid"), &mv)
        .await
        .unwrap();

    store.compact().await.unwrap();

    assert_eq!(store.count().await.unwrap(), 1);
    assert!(store.exists(&record.image_id).await.unwrap());
    let score = store.get_score(&record.image_id).await.unwrap();
    assert_eq!(score, Some((Score::new(0.8).expect("valid"), mv)));
}

#[tokio::test]
async fn summarise_returns_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = open_temp_store(&dir).await;
    let record = make_record("summ1");
    store.add(&record).await.unwrap();

    let mv = test_model_version();
    store
        .upsert_score(&record.image_id, &Score::new(0.7).expect("valid"), &mv)
        .await
        .unwrap();

    let summary = store.summarise().await.unwrap();
    assert_eq!(summary.image_count, 1);
    assert_eq!(summary.score_count, 1);
    assert_eq!(summary.scored_image_count, 1);
    assert!(summary.discovered_at_range.is_some());
    assert!(summary.original_at_range.is_some());
}
