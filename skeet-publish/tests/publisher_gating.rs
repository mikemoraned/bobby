//! Version-gating test: `FeedPublisher::publish_if_changed` only republishes
//! when a relevant table version (scores or appraisals) has moved.
//!
//! Requires Docker; the `_docker` test is filtered out by `just test-no-docker`.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use common::{CIDS, redis_url, scored_record, seed, v3, wait_ready};
use skeet_publish::{CdnImageUrlResolver, FeedPublisher, Limit, Order, PublishOutcome, connect};
use skeet_store::test_utils::open_temp_store;
use skeet_store::{ModelVersion, Score};
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
            &Score::new(0.8).expect("valid score"),
            &ModelVersion::from("test"),
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
