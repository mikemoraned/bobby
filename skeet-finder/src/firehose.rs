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
use shared::{SkeetId, SkeetImage};
use tracing::{info, warn};

pub async fn connect() -> Result<JetstreamReceiver, Box<dyn std::error::Error>> {
    info!("connecting to firehose");
    let config = JetstreamConfig {
        endpoint: DefaultJetstreamEndpoints::USEastOne.into(),
        compression: JetstreamCompression::Zstd,
        wanted_collections: vec!["app.bsky.feed.post".parse::<Nsid>()?],
        ..Default::default()
    };

    let jetstream = JetstreamConnector::new(config)?;
    Ok(jetstream.connect().await?)
}

/// If this event is a post creation with images, download each image and return them.
/// Returns an empty vec for non-image posts or non-create events.
pub async fn extract_skeet_images(
    event: &JetstreamEvent,
    http: &reqwest::Client,
) -> Vec<SkeetImage> {
    let JetstreamEvent::Commit(CommitEvent::Create { info, commit }) = event else {
        return Vec::new();
    };
    let KnownRecord::AppBskyFeedPost(post) = &commit.record else {
        return Vec::new();
    };

    if has_excluded_label(&post.data.labels) {
        return Vec::new();
    }

    let image_refs = extract_images(&post.data.embed);
    if image_refs.is_empty() {
        return Vec::new();
    }

    let did = info.did.as_str();
    let skeet_id = SkeetId::new(format!(
        "at://{}/app.bsky.feed.post/{}",
        did, commit.info.rkey
    ));
    let original_at = parse_created_at(&post.data.created_at);

    let mut results = Vec::new();
    for image_ref in &image_refs {
        let Some(cid) = blob_cid(&image_ref.data.image) else {
            warn!("skipping image with unrecognized blob ref format");
            continue;
        };

        let url = format!(
            "https://cdn.bsky.app/img/feed_fullsize/plain/{}/{}@jpeg",
            did, cid
        );

        let bytes = match http.get(&url).send().await {
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
            skeet_id: skeet_id.clone(),
            original_at,
            image: dynamic_image,
        });
    }

    results
}

const EXCLUDED_LABELS: &[&str] = &["porn", "sexual", "nudity", "!no-unauthenticated"];

fn has_excluded_label(labels: &Option<Union<RecordLabelsRefs>>) -> bool {
    let Some(Union::Refs(RecordLabelsRefs::ComAtprotoLabelDefsSelfLabels(self_labels))) = labels
    else {
        return false;
    };
    self_labels
        .values
        .iter()
        .any(|label| EXCLUDED_LABELS.contains(&label.val.as_str()))
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
