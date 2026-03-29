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
use shared::SkeetImage;
use shared::skeet_id::SkeetId;
use std::time::Duration;
use tracing::{info, warn};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

const ALL_ENDPOINTS: [DefaultJetstreamEndpoints; 4] = [
    DefaultJetstreamEndpoints::USEastOne,
    DefaultJetstreamEndpoints::USEastTwo,
    DefaultJetstreamEndpoints::USWestOne,
    DefaultJetstreamEndpoints::USWestTwo,
];

pub async fn connect() -> Result<JetstreamReceiver, Box<dyn std::error::Error>> {
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

/// A post that has images but hasn't been downloaded yet.
pub struct SkeetCandidate {
    pub skeet_id: SkeetId,
    pub original_at: DateTime<Utc>,
    pub image_urls: Vec<String>,
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

    let mut image_urls = Vec::new();
    for image_ref in &image_refs {
        let Some(cid) = blob_cid(&image_ref.data.image) else {
            warn!("skipping image with unrecognized blob ref format");
            continue;
        };
        image_urls.push(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/{}/{}@jpeg",
            did, cid
        ));
    }

    if image_urls.is_empty() {
        return None;
    }

    Some(SkeetCandidate {
        skeet_id,
        original_at,
        image_urls,
    })
}

/// Download the images for a candidate, returning a `SkeetImage` for each
/// that downloads and decodes successfully.
pub async fn download_candidate_images(
    candidate: &SkeetCandidate,
    http: &reqwest::Client,
) -> Vec<SkeetImage> {
    let mut results = Vec::new();

    for url in &candidate.image_urls {
        let bytes = match http.get(url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "failed to read image bytes");
                    continue;
                }
            },
            Ok(resp) => {
                warn!(status = %resp.status(), url, "image download failed");
                continue;
            }
            Err(e) => {
                warn!(error = %e, "image download request failed");
                continue;
            }
        };

        let dynamic_image = match image::load_from_memory(&bytes) {
            Ok(img) => img,
            Err(e) => {
                warn!(error = %e, "failed to decode downloaded image");
                continue;
            }
        };

        results.push(SkeetImage {
            skeet_id: candidate.skeet_id.clone(),
            original_at: candidate.original_at,
            image: dynamic_image,
        });
    }

    results
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

fn blob_cid(blob_ref: &BlobRef) -> Option<String> {
    match blob_ref {
        BlobRef::Typed(TypedBlobRef::Blob(blob)) => Some(blob.r#ref.0.to_string()),
        BlobRef::Untyped(untyped) => Some(untyped.cid.clone()),
    }
}

fn parse_created_at(dt: &atrium_api::types::string::Datetime) -> DateTime<Utc> {
    let fixed: &chrono::DateTime<chrono::FixedOffset> = dt.as_ref();
    fixed.with_timezone(&Utc)
}
