#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use cot::project::Bootstrapper;
use skeet_feed::StoreLayer;
use skeet_feed::feed_config::{FeedConfigLayer, FeedParams};
use skeet_feed::project::FeedProject;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Hostname for the feed generator (used in DID and service endpoint)
    #[arg(long)]
    hostname: String,

    /// Feed name identifier (used in the feed AT-URI)
    #[arg(long, default_value = "bobby-dev")]
    feed_name: String,

    /// Address to bind the server to
    #[arg(long, default_value = "0.0.0.0:8080")]
    bind: String,

    /// Maximum number of posts to return in the feed
    #[arg(long, default_value_t = 10)]
    max_entries: usize,

    /// Minimum score threshold for inclusion in the feed
    #[arg(long, default_value_t = 0.5)]
    min_score: f32,

    /// Maximum age in hours for posts to be included
    #[arg(long, default_value_t = 48)]
    max_age_hours: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let _guard = shared::tracing::init_with_file_and_stderr(
        "skeet_feed=info,shared=info,skeet_store=info",
        "feed.log",
        shared::tracing::TokioConsoleSupport::Disabled,
    );

    let store = args
        .store
        .open_store()
        .await
        .expect("failed to open store at startup");

    let feed_params = FeedParams {
        hostname: args.hostname.clone(),
        feed_name: args.feed_name,
        max_entries: args.max_entries,
        min_score: args.min_score,
        max_age_hours: args.max_age_hours,
    };

    info!(
        bind = %args.bind,
        hostname = %args.hostname,
        feed_uri = %feed_params.feed_uri(),
        "starting skeet-feed server"
    );

    let project = FeedProject {
        store_layer: StoreLayer::new(store),
        feed_config_layer: FeedConfigLayer::new(feed_params),
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, &args.bind).await?;
    Ok(())
}
