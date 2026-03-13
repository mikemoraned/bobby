#![warn(clippy::all, clippy::nursery)]

use atrium_api::{
    app::bsky::feed::post::RecordEmbedRefs, record::KnownRecord, types::Union,
};
use jetstream_oxide::{
    DefaultJetstreamEndpoints, JetstreamCompression, JetstreamConfig, JetstreamConnector,
    events::{JetstreamEvent, commit::CommitEvent},
    exports::Nsid,
};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skeet_finder=info".parse().expect("valid filter")),
        )
        .init();

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
    while let Ok(event) = receiver.recv_async().await {
        if let JetstreamEvent::Commit(CommitEvent::Create { info, commit }) = &event
            && let KnownRecord::AppBskyFeedPost(post) = &commit.record
        {
            post_count += 1;

            let image_count = count_images(&post.data.embed);
            if image_count > 0 {
                image_post_count += 1;
                info!(
                    posts = post_count,
                    image_posts = image_post_count,
                    image_count,
                    did = info.did.as_str(),
                    "post with images"
                );
            } else if post_count.is_multiple_of(500) {
                info!(
                    posts = post_count,
                    image_posts = image_post_count,
                    "progress"
                );
            }
        }
    }

    warn!("jetstream connection closed");
    Ok(())
}

fn count_images(embed: &Option<Union<RecordEmbedRefs>>) -> usize {
    let Some(Union::Refs(refs)) = embed else {
        return 0;
    };
    match refs {
        RecordEmbedRefs::AppBskyEmbedImagesMain(images) => images.images.len(),
        RecordEmbedRefs::AppBskyEmbedRecordWithMediaMain(record_with_media) => {
            if let Union::Refs(
                atrium_api::app::bsky::embed::record_with_media::MainMediaRefs::AppBskyEmbedImagesMain(images),
            ) = &record_with_media.media
            {
                images.images.len()
            } else {
                0
            }
        }
        _ => 0,
    }
}
