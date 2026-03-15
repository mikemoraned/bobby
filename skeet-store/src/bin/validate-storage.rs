#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;

use clap::Parser;
use skeet_store::SkeetStore;

#[derive(Parser)]
#[command(about = "Validate that the store is readable and writable")]
struct Args {
    #[arg(long)]
    store_path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let store = SkeetStore::open(&args.store_path).await?;
    store.validate().await?;

    eprintln!("Storage validation passed");
    Ok(())
}
