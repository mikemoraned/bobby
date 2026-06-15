#![warn(clippy::all, clippy::nursery)]

use std::sync::Arc;

use clap::Parser;
use cot::project::Bootstrapper;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_feed::{FeedSourceLayer, PublishedImagesSourceLayer};
use skeet_publish::{FeedSource, Limit, Order, PublishedImagesSource, RedisFeedSource};
use tracing::info;

#[derive(Parser)]
struct Args {
    /// Hostname for the feed generator (used in DID and service endpoint)
    #[arg(long)]
    hostname: String,

    /// DID of the Bluesky account that published the feed
    #[arg(long)]
    publisher_did: String,

    /// Feed name identifier (used in the feed AT-URI)
    #[arg(long, default_value = "bobby-dev")]
    feed_name: String,

    /// Address to bind the server to
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,

    /// Maximum number of posts to return in the feed
    #[arg(long, default_value_t = 10)]
    max_entries: usize,

    /// Redis URL for the publish server (env: BOBBY_REDIS_PUBLISH_URL)
    #[arg(long, env = "BOBBY_REDIS_PUBLISH_URL")]
    redis_publish_url: String,

    /// Site-specific Plausible analytics script URL. When set, the home page
    /// loads the Plausible script; omit it (staging/local) to load nothing.
    #[arg(long)]
    plausible_script_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The Upstash publish url is `rediss://`, so TLS runs through rustls — install
    // the process-global crypto provider once before any connection is made.
    #[allow(clippy::expect_used)]
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let args = Args::parse();

    let _guard = shared::tracing::init_with_file(
        "skeet_feed=info,skeet_publish=info,shared=info",
        "feed.log",
    );
    info!(git_hash = env!("BUILD_GIT_HASH"), "skeet-feed starting");

    let feed_params = FeedParams::new(
        args.hostname.clone(),
        args.publisher_did,
        args.feed_name,
        args.max_entries,
        args.plausible_script_url,
    );

    info!(
        bind = %args.bind,
        hostname = %args.hostname,
        feed_uri = %feed_params.feed_uri(),
        "starting skeet-feed server (feed from the redis publish server)"
    );

    // The Bluesky feed is the `quality-48h` list written by skeet-publish.
    let feed_source: Arc<dyn FeedSource> = Arc::new(RedisFeedSource::new(
        args.redis_publish_url.clone(),
        Order::Quality,
        Limit::hours(48),
    ));

    // The public image page renders the wider `quality-7d` list. Each published
    // image already carries its dimensions (measured by the publisher's CDN
    // probe), so the feed renders aspect ratios without fetching any image.
    let published_images_source: Arc<dyn PublishedImagesSource> = Arc::new(RedisFeedSource::new(
        args.redis_publish_url,
        Order::Quality,
        Limit::days(7),
    ));

    let project = FeedProject {
        feed_source_layer: FeedSourceLayer::new(feed_source),
        published_images_source_layer: PublishedImagesSourceLayer::new(published_images_source),
        feed_config_layer: FeedConfigLayer::new(feed_params),
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}
