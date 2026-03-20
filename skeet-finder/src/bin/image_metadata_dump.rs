#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::{ImageId, StoreArgs};
use tracing::info;

#[derive(Parser)]
#[command(about = "Look up an image in the store and dump its Bluesky post metadata")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    #[arg(long)]
    image_id: ImageId,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let store = args.store.open_store().await?;
    let stored = store
        .get_by_id(&args.image_id)
        .await?
        .ok_or_else(|| format!("no image found with id {}", args.image_id))?;

    info!(
        image_id = %stored.image_id,
        skeet_id = %stored.skeet_id,
        zone = %stored.zone,
        config_version = %stored.config_version,
        discovered_at = %stored.discovered_at,
        original_at = %stored.original_at,
        detected_text = ?stored.detected_text,
        image_size = %format_args!("{}x{}", stored.image.width(), stored.image.height()),
        "image metadata"
    );

    info!(at_uri = %stored.skeet_id, "fetching post thread");

    let http = reqwest::Client::new();
    let json = skeet_finder::metadata::fetch_post_thread(&http, &stored.skeet_id).await?;
    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}
