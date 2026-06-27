#![warn(clippy::all, clippy::nursery)]

use chrono::{Duration, Utc};
use clap::Parser;
use skeet_store::{Statistics, StoreArgs};
use tracing::info;

#[derive(Parser)]
#[command(
    name = "show-prune-statistics",
    about = "Summarise prune statistics (skeets seen, images examined/saved) over a recent interval"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// How far back from now to summarise, in minutes.
    #[arg(long, default_value = "5")]
    minutes: i64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("warn");
    info!(
        git_hash = env!("BUILD_GIT_HASH"),
        "show-prune-statistics starting"
    );

    let args = Args::parse();
    let store = args.store.open_store("show-prune-statistics").await?;

    let end = Utc::now();
    let start = end - Duration::minutes(args.minutes);
    let stats = store.interval_counts(start, end).await?;

    println!(
        "interval:        {} -> {}",
        stats.interval_start, stats.interval_end
    );
    println!("skeets seen:     {}", stats.skeets_seen);
    println!("images examined: {}", stats.images_examined);
    println!("images saved:    {}", stats.images_saved);
    Ok(())
}
