#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use cot::project::Bootstrapper;
use shared::RefineModels;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_feed::FeedSourceLayer;
use skeet_publish::{FeedCache, FeedSource, Limit, LiveFeedSource, Order, RedisFeedSource};
use skeet_store::StoreArgs;
use tracing::info;

/// Where `getFeedSkeleton` reads the feed from.
#[derive(Clone, Copy, clap::ValueEnum)]
enum FeedSourceKind {
    /// Compute the feed live from the store via `FeedCache`.
    Library,
    /// Read the published `recency-48h` list from the redis publish server.
    Redis,
}

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

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

    /// Maximum age in hours for posts to be included
    #[arg(long, default_value_t = 48)]
    max_age_hours: u64,

    /// Path to the refine model registry (refine.toml)
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// Which feed source to serve `getFeedSkeleton` from
    #[arg(long, value_enum, default_value = "library")]
    feed_source: FeedSourceKind,

    /// Redis URL for the publish server (env: BOBBY_REDIS_PUBLISH_URL).
    /// Required with `--feed-source redis`.
    #[arg(long, env = "BOBBY_REDIS_PUBLISH_URL")]
    redis_publish_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Installs the process-global default exactly once at startup, so it cannot already
    // be set; the `expect` keeps that failure reason explicit.
    #[allow(clippy::expect_used)]
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let args = Args::parse();

    let _guard =
        shared::tracing::init_with_file("skeet_feed=info,shared=info,skeet_store=info", "feed.log");
    info!(git_hash = env!("BUILD_GIT_HASH"), "skeet-feed starting");

    let feed_params = FeedParams {
        hostname: args.hostname.clone(),
        publisher_did: args.publisher_did,
        feed_name: args.feed_name,
        max_entries: args.max_entries,
        max_age_hours: args.max_age_hours,
    };

    info!(
        bind = %args.bind,
        hostname = %args.hostname,
        feed_uri = %feed_params.feed_uri(),
        "starting skeet-feed server"
    );

    let feed_source: Arc<dyn FeedSource> = match args.feed_source {
        FeedSourceKind::Library => {
            let store = Arc::new(args.store.open_store("feed").await?);
            let models = Arc::new(
                RefineModels::load(&args.model_path)
                    .unwrap_or_else(|e| panic!("failed to load {}: {e}", args.model_path.display())),
            );
            info!(path = %args.model_path.display(), "loaded refine models");
            let cache = Arc::new(FeedCache::new(
                store,
                models,
                feed_params.max_entries,
                feed_params.max_age_hours,
            ));
            cache.spawn_background_refresh();
            info!("serving feed from the live store (library)");
            Arc::new(LiveFeedSource::new(cache))
        }
        FeedSourceKind::Redis => {
            let url = args.redis_publish_url.ok_or(
                "--redis-publish-url (env BOBBY_REDIS_PUBLISH_URL) is required with --feed-source redis",
            )?;
            info!("serving feed from the redis publish server");
            // The Bluesky feed is the `recency-48h` list written by skeet-publish.
            Arc::new(RedisFeedSource::new(url, Order::Recency, Limit::hours(48)))
        }
    };

    let project = FeedProject {
        feed_source_layer: FeedSourceLayer::new(feed_source),
        feed_config_layer: FeedConfigLayer::new(feed_params),
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}
