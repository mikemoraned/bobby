#![warn(clippy::all, clippy::nursery)]

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use cot::project::Bootstrapper;
use skeet_feed::{FeedConfigLayer, FeedParams};
use skeet_feed::preview::selection::select_tiles;
use skeet_feed::preview::state::{PreviewState, PreviewStateLayer};
use skeet_feed::preview::{PREVIEW_HEIGHT, PREVIEW_WIDTH, generate_montage};
use skeet_feed::FeedProject;
use skeet_feed::{FeedSourceLayer, PublishedImagesSourceLayer};
use skeet_publish::{FallbackFeedSource, FeedSource, Limit, Order, PublishedImagesSource};
use tracing::{info, warn};

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
        env!("CARGO_CRATE_NAME"),
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
    )?;

    info!(
        bind = %args.bind,
        hostname = %args.hostname,
        feed_uri = %feed_params.feed_uri(),
        "starting skeet-feed server (feed from the redis publish server)"
    );

    // The Bluesky feed prefers the `quality,recency-48h` list written by
    // skeet-publish, falling back to successively wider same-order lists
    // (`quality,recency-7d`, …) when it is empty or missing, so an outage degrades
    // gracefully to older data.
    let feed_source: Arc<dyn FeedSource> = Arc::new(FallbackFeedSource::new(
        args.redis_publish_url.clone(),
        Order::QualityRecency,
        Limit::hours(48),
    ));

    // The public image page prefers the wider `quality,recency-12w` list, falling
    // back to wider same-order lists when it is empty or missing. Each published
    // image already carries its dimensions (measured by the publisher's CDN probe),
    // so the feed renders aspect ratios without fetching any image.
    let published_images_source: Arc<dyn PublishedImagesSource> = Arc::new(FallbackFeedSource::new(
        args.redis_publish_url,
        Order::QualityRecency,
        Limit::weeks(12),
    ));

    let preview_state = Arc::new(PreviewState::new(
        reqwest::Client::new(),
        (PREVIEW_WIDTH, PREVIEW_HEIGHT),
    ));
    // Warm the montage cache before serving so the first scraper sees a real
    // image, but cap the wait so a slow CDN can't stall startup. Non-fatal: on
    // failure or timeout we start with an empty cache and the first request
    // regenerates.
    let warm_up = warm_preview_cache(&preview_state, published_images_source.as_ref());
    if tokio::time::timeout(WARM_UP_TIMEOUT, warm_up).await.is_err() {
        warn!(
            timeout_secs = WARM_UP_TIMEOUT.as_secs(),
            "preview cache warm-up timed out; starting with an empty cache"
        );
    }

    let project = FeedProject {
        feed_source_layer: FeedSourceLayer::new(feed_source),
        published_images_source_layer: PublishedImagesSourceLayer::new(published_images_source),
        feed_config_layer: FeedConfigLayer::new(feed_params),
        preview_state_layer: PreviewStateLayer::new(preview_state),
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}

/// How long startup waits for the montage cache to warm before serving anyway.
const WARM_UP_TIMEOUT: Duration = Duration::from_secs(30);

/// Generate the montage once at startup and populate the cache. Non-fatal: a
/// read or generation failure leaves the cache empty and serving continues.
async fn warm_preview_cache(state: &PreviewState, source: &dyn PublishedImagesSource) {
    match source.published_images().await {
        Ok(published) => {
            let selection = select_tiles(&published.images);
            state
                .cache
                .warm(selection.signature, || async {
                    generate_montage(&state.fetcher, selection.tile_urls, state.size).await
                })
                .await;
            info!("preview montage cache warmed");
        }
        Err(error) => {
            warn!(%error, "preview cache warm-up skipped; starting with an empty cache");
        }
    }
}
