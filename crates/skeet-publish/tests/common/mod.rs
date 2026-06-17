//! Shared fixtures for the `skeet-publish` redis integration tests: a temp store
//! seeded with `V3` scored skeets, plus testcontainers redis helpers.

use std::time::Duration;

use chrono::Utc;
use shared::{BlueskyCid, ImageId};
use skeet_publish::connect;
use skeet_store::test_utils::test_image;
use skeet_store::{DiscoveredAt, ImageRecord, ModelVersion, OriginalAt, Score, SkeetStore, Zone};
use testcontainers::ContainerAsync;
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tokio::time::{Instant, sleep};

/// Distinct, valid CIDv1 (raw, sha2-256) strings — content is irrelevant; we
/// only need distinct `V3` image ids so the CDN resolver succeeds for each.
pub const CIDS: [&str; 5] = [
    "bafkreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiabaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiacaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiadaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "bafkreiaeaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
];

pub fn v3(i: usize) -> ImageId {
    ImageId::V3(BlueskyCid::new(CIDS[i]).expect("valid cid"))
}

pub fn scored_record(rkey: &str, image_id: ImageId) -> ImageRecord {
    ImageRecord {
        image_id,
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse()
            .expect("valid skeet id"),
        image: test_image(),
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    }
}

/// Seed three visible skeets (score ≥ 0.5 for the `test` model) and one hidden
/// (score < 0.5), all `V3` and published now.
pub async fn seed(store: &SkeetStore) {
    let mv = ModelVersion::from("test");
    let rows = [
        ("vis1", 0, 0.9),
        ("vis2", 1, 0.6),
        ("vis3", 2, 0.55),
        ("hidden", 3, 0.1),
    ];
    for (rkey, cid_idx, score) in rows {
        let record = scored_record(rkey, v3(cid_idx));
        store.add(&record).await.expect("add record");
        store
            .upsert_score(&record.image_id, &Score::new(score).expect("valid score"), &mv)
            .await
            .expect("upsert score");
    }
}

pub async fn redis_url(container: &ContainerAsync<Redis>) -> String {
    let host = container.get_host().await.expect("host");
    let port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("mapped port");
    format!("redis://{host}:{port}")
}

/// Wait until the container actually answers before using it (Docker's host
/// port-forward can refuse briefly after the container reports ready).
pub async fn wait_ready(url: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if connect(url).await.is_ok() {
            return;
        }
        assert!(Instant::now() < deadline, "redis not ready within 30s");
        sleep(Duration::from_millis(100)).await;
    }
}
