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

    let args = Args::parse();
    let store = args.store.open_store().await?;

    info!("generating summary");
    let summary = store.summarise().await?;
    println!("{summary}");

    Ok(())
}
