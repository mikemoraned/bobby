#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
#[command(about = "Show a summary of what a store contains")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");
    info!(git_hash = env!("BUILD_GIT_HASH"), "summarise starting");

    let args = Args::parse();
    let store = args.store.open_store("summarise").await?;

    info!("generating summary");
    let summary = store.summarise().await?;
    println!("{summary}");

    Ok(())
}
