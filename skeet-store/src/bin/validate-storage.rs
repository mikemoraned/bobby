#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::StoreArgs;

#[derive(Parser)]
#[command(about = "Validate that the store is readable and writable")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let store = args.store.open_store().await?;
    store.validate().await?;

    eprintln!("Storage validation passed");
    Ok(())
}
