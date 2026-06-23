use atrium_api::{
    app::bsky::{
        embed::{images::Image, record_with_media::MainMediaRefs},
        feed::post::{RecordEmbedRefs, RecordLabelsRefs},
    },
    record::KnownRecord,
    types::{BlobRef, TypedBlobRef, Union},
};
use chrono::{DateTime, Utc};
use jetstream_oxide::{
    DefaultJetstreamEndpoints, JetstreamCompression, JetstreamConfig, JetstreamConnector,
    JetstreamReceiver,
    events::{JetstreamEvent, commit::CommitEvent},
    exports::Nsid,
};
use shared::skeet_id::SkeetId;
use shared::{BlueskyCid, SkeetImage};
use std::time::Duration;
use tracing::{info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How far to rewind the resume cursor before reconnecting, so the replayed
/// window overlaps the disconnect boundary rather than leaving a gap. Safe
/// because the ultimate sink is idempotent (keyed by content hash / `SkeetId`).
const RECONNECT_REWIND: Duration = Duration::from_secs(5);

/// The `time_us` carried by any Jetstream event, regardless of variant. Used to
/// advance the resume cursor on every observed event — not only post candidates
/// — so a quiet period can't leave the cursor stale.
pub const fn event_time_us(event: &JetstreamEvent) -> u64 {
    match event {
        JetstreamEvent::Commit(
            CommitEvent::Create { info, .. }
            | CommitEvent::Update { info, .. }
            | CommitEvent::Delete { info, .. },
        ) => info.time_us,
        JetstreamEvent::Identity(identity) => identity.info.time_us,
        JetstreamEvent::Account(account) => account.info.time_us,
    }
}

/// Build a resume cursor from the last observed event time, rewound by
/// [`RECONNECT_REWIND`]. Returns `None` when the rewind would precede the Unix
/// epoch (no meaningful cursor to resume from).
pub const fn cursor_from(time_us: u64) -> Option<DateTime<Utc>> {
    let rewind_micros = RECONNECT_REWIND.as_micros() as u64;
    match time_us.checked_sub(rewind_micros) {
        Some(micros) => DateTime::from_timestamp_micros(micros as i64),
        None => None,
    }
}

const ALL_ENDPOINTS: [DefaultJetstreamEndpoints; 4] = [
    DefaultJetstreamEndpoints::USEastOne,
    DefaultJetstreamEndpoints::USEastTwo,
    DefaultJetstreamEndpoints::USWestOne,
    DefaultJetstreamEndpoints::USWestTwo,
];

pub async fn connect(
    cursor: Option<DateTime<Utc>>,
) -> Result<JetstreamReceiver, Box<dyn std::error::Error>> {
    info!("connecting to firehose");

    let wanted_collections = vec!["app.bsky.feed.post".parse::<Nsid>()?];

    let mut endpoints: Vec<String> = ALL_ENDPOINTS.map(Into::into).to_vec();
    fastrand::shuffle(&mut endpoints);

    for endpoint_str in &endpoints {
        info!(endpoint = %endpoint_str, "trying endpoint");

        let config = JetstreamConfig {
            endpoint: endpoint_str.clone(),
            compression: JetstreamCompression::Zstd,
            wanted_collections: wanted_collections.clone(),
            max_retries: 0,
            cursor,
            ..Default::default()
        };

        let connector = JetstreamConnector::new(config)?;
        match tokio::time::timeout(CONNECT_TIMEOUT, connector.connect()).await {
            Ok(Ok(receiver)) => {
                info!(endpoint = %endpoint_str, "connected to firehose");
                return Ok(receiver);
            }
            Ok(Err(e)) => {
                warn!(endpoint = %endpoint_str, error = %e, "connection failed");
            }
            Err(_) => {
                warn!(endpoint = %endpoint_str, "connection timed out after {:?}", CONNECT_TIMEOUT);
            }
        }
    }

    Err(format!(
        "failed to connect to any firehose endpoint after trying all {} endpoints",
        ALL_ENDPOINTS.len()
    )
    .into())
}

/// One image of a post: its blob CID and the CDN URL to fetch it from.
pub struct ImageCandidate {
    pub cid: BlueskyCid,
    pub url: String,
}

/// A post that has images but hasn't been downloaded yet.
pub struct SkeetCandidate {
    pub skeet_id: SkeetId,
    pub original_at: DateTime<Utc>,
    pub images: Vec<ImageCandidate>,
}

/// If this event is a post creation with images, extract the candidate info
/// (skeet id + image URLs) without downloading. Returns `None` for non-image
/// posts or non-create events.
pub fn extract_skeet_candidate(event: &JetstreamEvent) -> Option<SkeetCandidate> {
    let JetstreamEvent::Commit(CommitEvent::Create { info, commit }) = event else {
        return None;
    };
    let KnownRecord::AppBskyFeedPost(post) = &commit.record else {
        return None;
    };

    if has_excluded_label(&post.data.labels) {
        return None;
    }

    let image_refs = extract_images(&post.data.embed);
    if image_refs.is_empty() {
        return None;
    }

    let did = info.did.as_str();
    let skeet_id = SkeetId::for_post(did, &commit.info.rkey);
    let original_at = parse_created_at(&post.data.created_at);

    let images: Vec<ImageCandidate> = image_refs
        .iter()
        .filter_map(|image_ref| image_candidate(did, &image_ref.data.image))
        .collect();

    if images.is_empty() {
        return None;
    }

    Some(SkeetCandidate {
        skeet_id,
        original_at,
        images,
    })
}

/// Build the CDN URL + carry the blob CID for one image, or `None` if the blob
/// ref doesn't yield a parseable CID.
fn image_candidate(did: &str, blob_ref: &BlobRef) -> Option<ImageCandidate> {
    let Some(cid) = blob_cid(blob_ref) else {
        warn!("skipping image with unrecognized blob ref or CID");
        return None;
    };
    let url = bluesky::bsky_cdn_thumbnail_url(did, &cid.to_string());
    Some(ImageCandidate { cid, url })
}

/// Download the images for a candidate, returning a `SkeetImage` for each
/// that downloads and decodes successfully. Downloads all images concurrently.
pub async fn download_candidate_images(
    candidate: &SkeetCandidate,
    http: &reqwest::Client,
) -> Vec<SkeetImage> {
    let mut set = tokio::task::JoinSet::new();

    for image in &candidate.images {
        let http = http.clone();
        let url = image.url.clone();
        let cid = image.cid.clone();
        let skeet_id = candidate.skeet_id.clone();
        let original_at = candidate.original_at;
        set.spawn(
            async move { download_single_image(&http, &url, cid, skeet_id, original_at).await },
        );
    }

    let mut results = Vec::new();
    while let Some(Ok(Some(image))) = set.join_next().await {
        results.push(image);
    }
    results
}

async fn download_single_image(
    http: &reqwest::Client,
    url: &str,
    cid: BlueskyCid,
    skeet_id: SkeetId,
    original_at: chrono::DateTime<chrono::Utc>,
) -> Option<SkeetImage> {
    let bytes = match http.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "failed to read image bytes");
                return None;
            }
        },
        Ok(resp) => {
            warn!(status = %resp.status(), url, "image download failed");
            return None;
        }
        Err(e) => {
            warn!(error = %e, "image download request failed");
            return None;
        }
    };

    match image::load_from_memory(&bytes) {
        Ok(img) => Some(SkeetImage {
            skeet_id,
            original_at,
            image: img,
            cid,
        }),
        Err(e) => {
            warn!(error = %e, "failed to decode downloaded image");
            None
        }
    }
}

fn has_excluded_label(labels: &Option<Union<RecordLabelsRefs>>) -> bool {
    let Some(Union::Refs(RecordLabelsRefs::ComAtprotoLabelDefsSelfLabels(self_labels))) = labels
    else {
        return false;
    };
    self_labels
        .values
        .iter()
        .any(|label| shared::labels::EXCLUDED_VALUES.contains(&label.val.as_str()))
}

fn extract_images(embed: &Option<Union<RecordEmbedRefs>>) -> Vec<&Image> {
    let Some(Union::Refs(refs)) = embed else {
        return Vec::new();
    };
    match refs {
        RecordEmbedRefs::AppBskyEmbedImagesMain(images) => images.images.iter().collect(),
        RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(record_with_media) => {
            if let Union::Refs(MainMediaRefs::AppBskyEmbedImagesMain(images)) =
                &record_with_media.media
            {
                images.images.iter().collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn blob_cid(blob_ref: &BlobRef) -> Option<BlueskyCid> {
    let cid_str = match blob_ref {
        BlobRef::Typed(TypedBlobRef::Blob(blob)) => blob.r#ref.0.to_string(),
        BlobRef::Untyped(untyped) => untyped.cid.clone(),
    };
    BlueskyCid::new(cid_str).ok()
}

fn parse_created_at(dt: &atrium_api::types::string::Datetime) -> DateTime<Utc> {
    let fixed: &chrono::DateTime<chrono::FixedOffset> = dt.as_ref();
    fixed.with_timezone(&Utc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Largest `time_us` whose rewound cursor is still a representable
    // `DateTime<Utc>`; keeps the proptests away from chrono's range edge.
    const MAX_REPR_US: u64 = 8_000_000_000_000_000_000;

    fn event_from_json(json: &str) -> JetstreamEvent {
        serde_json::from_str(json).expect("valid jetstream event json")
    }

    #[test]
    fn event_time_us_reads_commit_create() {
        let json = r#"{
            "did": "did:plc:abc123",
            "time_us": 1111,
            "kind": "commit",
            "commit": {
                "operation": "create",
                "rev": "rev1",
                "rkey": "rkey1",
                "collection": "app.bsky.feed.post",
                "cid": "bafyreibvjvcv745gig4mvqs4hctx4zfkono4rjejm2ta6gtyay3sefw7p4",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "hello",
                    "createdAt": "2024-01-01T00:00:00.000Z"
                }
            }
        }"#;
        assert_eq!(event_time_us(&event_from_json(json)), 1111);
    }

    #[test]
    fn event_time_us_reads_commit_delete() {
        let json = r#"{
            "did": "did:plc:abc123",
            "time_us": 2222,
            "kind": "commit",
            "commit": {
                "operation": "delete",
                "rev": "rev1",
                "rkey": "rkey1",
                "collection": "app.bsky.feed.post"
            }
        }"#;
        assert_eq!(event_time_us(&event_from_json(json)), 2222);
    }

    #[test]
    fn event_time_us_reads_identity() {
        let json = r#"{
            "did": "did:plc:abc123",
            "time_us": 3333,
            "kind": "identity",
            "identity": {
                "did": "did:plc:abc123",
                "handle": "alice.test",
                "seq": 1,
                "time": "2024-01-01T00:00:00.000Z"
            }
        }"#;
        assert_eq!(event_time_us(&event_from_json(json)), 3333);
    }

    #[test]
    fn event_time_us_reads_account() {
        let json = r#"{
            "did": "did:plc:abc123",
            "time_us": 4444,
            "kind": "account",
            "account": {
                "active": true,
                "did": "did:plc:abc123",
                "seq": 1,
                "time": "2024-01-01T00:00:00.000Z"
            }
        }"#;
        assert_eq!(event_time_us(&event_from_json(json)), 4444);
    }

    #[test]
    fn cursor_from_rewinds_by_the_configured_amount() {
        let time_us = 1_700_000_000_000_000;
        let cursor = cursor_from(time_us).expect("representable");
        let rewind_micros = RECONNECT_REWIND.as_micros() as i64;
        assert_eq!(cursor.timestamp_micros(), time_us as i64 - rewind_micros);
    }

    #[test]
    fn cursor_from_undefined_if_would_be_negative() {
        assert_eq!(cursor_from(0), None);
        let just_below_rewind = RECONNECT_REWIND.as_micros() as u64 - 1;
        assert_eq!(cursor_from(just_below_rewind), None);
    }

    proptest! {
        #[test]
        fn cursor_is_not_after_the_event_instant(
            time_us in (RECONNECT_REWIND.as_micros() as u64)..=MAX_REPR_US
        ) {
            let cursor = cursor_from(time_us).expect("representable");
            prop_assert!(cursor.timestamp_micros() <= time_us as i64);
        }

        #[test]
        fn cursor_is_monotonic_in_time_us(
            a in (RECONNECT_REWIND.as_micros() as u64)..=MAX_REPR_US,
            b in (RECONNECT_REWIND.as_micros() as u64)..=MAX_REPR_US,
        ) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let c_lo = cursor_from(lo).expect("representable");
            let c_hi = cursor_from(hi).expect("representable");
            prop_assert!(c_lo.timestamp_micros() <= c_hi.timestamp_micros());
        }

        #[test]
        fn cursor_round_trips_micros_within_the_rewind(
            time_us in (RECONNECT_REWIND.as_micros() as u64)..=MAX_REPR_US
        ) {
            let cursor = cursor_from(time_us).expect("representable");
            let rewind_micros = RECONNECT_REWIND.as_micros() as i64;
            prop_assert_eq!(cursor.timestamp_micros(), time_us as i64 - rewind_micros);
        }
    }
}
