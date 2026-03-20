#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
#[command(about = "Validate that the store is readable and writable")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let store = args.store.open_store().await?;
    store.validate().await?;

    info!("storage validation passed");
    Ok(())
}
