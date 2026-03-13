#![warn(clippy::all, clippy::nursery)]

use atrium_api::record::KnownRecord;
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

    let mut event_count: u64 = 0;
    let mut post_count: u64 = 0;
    while let Ok(event) = receiver.recv_async().await {
        event_count += 1;

        if let JetstreamEvent::Commit(CommitEvent::Create { info, commit }) = &event
            && let KnownRecord::AppBskyFeedPost(post) = &commit.record
        {
            post_count += 1;
            let has_embed = post.data.embed.is_some();
            if post_count.is_multiple_of(100) {
                info!(
                    events = event_count,
                    posts = post_count,
                    did = info.did.as_str(),
                    has_embed,
                    "post received"
                );
            }
        }
    }

    warn!("jetstream connection closed");
    Ok(())
}
