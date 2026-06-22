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
