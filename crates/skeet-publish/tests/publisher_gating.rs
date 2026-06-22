//! Version-gating test: `FeedPublisher::publish_if_changed` only republishes
//! when a relevant table version (scores or appraisals) has moved.
//!
//! Requires Docker; the `_docker` test is filtered out by `just test-no-docker`.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use bluesky::StaticExistenceChecker;
use chrono::Utc;
use common::{CIDS, redis_url, scored_record, seed, v3, wait_ready};
use deadpool_redis::redis;
use shared::SkeetId;
use skeet_publish::{
    CdnImageUrlResolver, FeedPublisher, Limit, Order, PublishOutcome, PublishedList, connect,
};
use skeet_store::test_utils::open_temp_store;
use skeet_store::{Images, ModelScore, ModelVersion, Score, Scores};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::Redis;

/// Validates the shared `CIDS` fixtures parse to distinct `V3` ids, without
/// Docker (so a bad fixture fails fast in `just test-no-docker`).
#[test]
fn hardcoded_cids_are_valid_and_distinct() {
    let ids: HashSet<String> = (0..CIDS.len()).map(|i| v3(i).to_string()).collect();
    assert_eq!(ids.len(), CIDS.len(), "cids must parse and be distinct");
}

#[tokio::test]
async fn skips_publish_when_no_relevant_change_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed(&store).await;
    let models = test_support::test_models();
    let spec = (Order::Recency, Limit::hours(48));

    let container = Redis::default().start().await.expect("start redis");
    let url = redis_url(&container).await;
    wait_ready(&url).await;

    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        models,
        Arc::new(CdnImageUrlResolver),
        Arc::new(StaticExistenceChecker::all_present()),
        vec![spec],
    );
    let mut conn = connect(&url).await.expect("connect");

    // First cycle publishes; an immediate second sees no relevant change.
    assert!(matches!(
        publisher
            .publish_if_changed(&mut conn, Utc::now())
            .await
            .expect("first cycle"),
        PublishOutcome::Published(_)
    ));
    assert!(matches!(
        publisher
            .publish_if_changed(&mut conn, Utc::now())
            .await
            .expect("second cycle"),
        PublishOutcome::Unchanged
    ));

    // A new score moves the scores table → the next cycle republishes.
    let extra = scored_record("extra", v3(4));
    store.add(&extra).await.expect("add extra");
    store
        .upsert_score(
            &extra.image_id,
            ModelScore {
                score: Score::new(0.8).expect("valid score"),
                model_version: ModelVersion::from("test"),
            },
        )
        .await
        .expect("upsert extra score");
    assert!(matches!(
        publisher
            .publish_if_changed(&mut conn, Utc::now())
            .await
            .expect("third cycle"),
        PublishOutcome::Published(_)
    ));
}

#[tokio::test]
async fn publish_writes_existence_flags_from_checker_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed(&store).await;
    let models = test_support::test_models();
    let spec = (Order::Recency, Limit::hours(48));
    let list = PublishedList::new(spec.0, spec.1);

    let container = Redis::default().start().await.expect("start redis");
    let url = redis_url(&container).await;
    wait_ready(&url).await;

    // The double marks one seeded skeet missing; everything else is present.
    let missing: SkeetId = "at://did:plc:abc/app.bsky.feed.post/vis2"
        .parse()
        .expect("valid skeet id");
    let checker = StaticExistenceChecker::all_present().with_missing_skeets([missing.clone()]);

    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        models,
        Arc::new(CdnImageUrlResolver),
        Arc::new(checker),
        vec![spec],
    );
    let mut conn = connect(&url).await.expect("connect");
    publisher
        .publish(&mut conn, Utc::now())
        .await
        .expect("publish");

    let pairs = list.read(&mut conn).await.expect("read");
    assert!(
        pairs.iter().any(|p| p.skeet_id == missing),
        "the missing skeet stays in the stored list, just flagged"
    );
    for p in &pairs {
        assert_eq!(
            p.skeet_id_exists,
            p.skeet_id != missing,
            "skeet_id_exists flag for {}",
            p.skeet_id
        );
        assert!(p.image_url_exists, "all images reported present");
    }
}

#[tokio::test]
async fn republishes_when_the_list_was_deleted_docker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(open_temp_store(&dir).await);
    seed(&store).await;
    let models = test_support::test_models();
    let spec = (Order::Recency, Limit::hours(48));
    let list = PublishedList::new(spec.0, spec.1);

    let container = Redis::default().start().await.expect("start redis");
    let url = redis_url(&container).await;
    wait_ready(&url).await;

    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        models,
        Arc::new(CdnImageUrlResolver),
        Arc::new(StaticExistenceChecker::all_present()),
        vec![spec],
    );
    let mut conn = connect(&url).await.expect("connect");

    // First publish creates a non-empty list; the next cycle skips (nothing moved).
    assert!(matches!(
        publisher
            .publish_if_changed(&mut conn, Utc::now())
            .await
            .expect("first cycle"),
        PublishOutcome::Published(_)
    ));
    assert!(!list.read(&mut conn).await.expect("read").is_empty());
    assert!(matches!(
        publisher
            .publish_if_changed(&mut conn, Utc::now())
            .await
            .expect("second cycle"),
        PublishOutcome::Unchanged
    ));

    // The list vanishes out-of-band (eviction / flush / manual delete).
    redis::cmd("DEL")
        .arg(list.name())
        .exec_async(&mut conn)
        .await
        .expect("delete list");
    assert!(list.read(&mut conn).await.expect("read").is_empty());

    // Even with no store change, the next cycle republishes to restore it.
    assert!(matches!(
        publisher
            .publish_if_changed(&mut conn, Utc::now())
            .await
            .expect("third cycle"),
        PublishOutcome::Published(_)
    ));
    assert!(
        !list.read(&mut conn).await.expect("read").is_empty(),
        "the deleted list should be restored"
    );
}
