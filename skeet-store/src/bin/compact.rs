#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
#[command(about = "Force-compact a LanceDB store to reduce fragment count")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();
    let store = args.store.open_store().await?;

    info!("starting compaction");
    store.compact().await?;
    info!("compaction finished");

    Ok(())
}
