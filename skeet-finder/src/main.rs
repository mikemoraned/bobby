#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;

use atrium_api::{
    app::bsky::{
        embed::{images::Image, record_with_media::MainMediaRefs},
        feed::post::RecordEmbedRefs,
    },
    record::KnownRecord,
    types::{BlobRef, TypedBlobRef, Union},
};
use chrono::{DateTime, Utc};
use jetstream_oxide::{
    DefaultJetstreamEndpoints, JetstreamCompression, JetstreamConfig, JetstreamConnector,
    events::{JetstreamEvent, commit::CommitEvent},
    exports::Nsid,
};
use rand::Rng;
use skeet_store::{DiscoveredAt, ImageId, ImageRecord, OriginalAt, SkeetId, SkeetStore};
use tracing::{info, warn};

const SELECTION_PROBABILITY: f64 = 0.01;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skeet_finder=info".parse().expect("valid filter")),
        )
        .init();

    let store_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: skeet-finder <store-path>")?;

    let store = SkeetStore::open(&store_path).await?;
    let http = reqwest::Client::new();
    let mut rng = rand::rng();

    let config = JetstreamConfig {
        endpoint: DefaultJetstreamEndpoints::USEastOne.into(),
        compression: JetstreamCompression::Zstd,
        wanted_collections: vec!["app.bsky.feed.post".parse::<Nsid>()?],
        ..Default::default()
    };

    let jetstream = JetstreamConnector::new(config)?;
    let receiver = jetstream.connect().await?;

    info!("connected to jetstream, listening for posts...");

    let mut post_count: u64 = 0;
    let mut image_post_count: u64 = 0;
    let mut saved_count: u64 = 0;
    while let Ok(event) = receiver.recv_async().await {
        if let JetstreamEvent::Commit(CommitEvent::Create { info, commit }) = &event
            && let KnownRecord::AppBskyFeedPost(post) = &commit.record
        {
            post_count += 1;

            let images = extract_images(&post.data.embed);
            if images.is_empty() {
                if post_count.is_multiple_of(500) {
                    info!(
                        posts = post_count,
                        image_posts = image_post_count,
                        saved = saved_count,
                        "progress"
                    );
                }
                continue;
            }

            image_post_count += 1;

            if !rng.random_bool(SELECTION_PROBABILITY) {
                continue;
            }

            let did = info.did.as_str();
            let skeet_id = SkeetId::new(format!(
                "at://{}/app.bsky.feed.post/{}",
                did, commit.info.rkey
            ));
            let original_at = parse_created_at(&post.data.created_at);

            for image_ref in &images {
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

                let record = ImageRecord {
                    image_id: ImageId::new(),
                    skeet_id: skeet_id.clone(),
                    image: dynamic_image,
                    discovered_at: DiscoveredAt::now(),
                    original_at: OriginalAt::new(original_at),
                };

                match store.add(&record).await {
                    Ok(()) => {
                        saved_count += 1;
                        info!(
                            saved = saved_count,
                            image_posts = image_post_count,
                            posts = post_count,
                            skeet_id = %skeet_id,
                            "saved image"
                        );
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to save image to store");
                    }
                }
            }
        }
    }

    warn!("jetstream connection closed");
    Ok(())
}

fn extract_images(embed: &Option<Union<RecordEmbedRefs>>) -> Vec<&Image> {
    let Some(Union::Refs(refs)) = embed else {
        return Vec::new();
    };
    match refs {
        RecordEmbedRefs::AppBskyEmbedImagesMain(images) => {
            images.images.iter().collect()
        }
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
