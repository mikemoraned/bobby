#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use cot::project::Bootstrapper;
use shared::RefineModels;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_feed::FeedSourceLayer;
use skeet_publish::{FeedCache, FeedSource, LiveFeedSource};
use skeet_store::StoreArgs;
use tracing::info;

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let args = Args::parse();

    let _guard =
        shared::tracing::init_with_file("skeet_feed=info,shared=info,skeet_store=info", "feed.log");
    info!(git_hash = env!("BUILD_GIT_HASH"), "skeet-feed starting");

    let store = Arc::new(
        args.store
            .open_store("feed")
            .await
            .expect("failed to open store at startup"),
    );

    let models = Arc::new(
        RefineModels::load(&args.model_path)
            .unwrap_or_else(|e| panic!("failed to load {}: {e}", args.model_path.display())),
    );
    info!(path = %args.model_path.display(), "loaded refine models");

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

    let cache = Arc::new(FeedCache::new(
        Arc::clone(&store),
        Arc::clone(&models),
        feed_params.max_entries,
        feed_params.max_age_hours,
    ));
    cache.spawn_background_refresh();

    let feed_source: Arc<dyn FeedSource> = Arc::new(LiveFeedSource::new(Arc::clone(&cache)));

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
