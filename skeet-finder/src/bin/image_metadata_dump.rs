#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::{ImageId, StoreArgs};

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
    let args = Args::parse();

    let store = args.store.open_store().await?;
    let stored = store
        .get_by_id(&args.image_id)
        .await?
        .ok_or_else(|| format!("no image found with id {}", args.image_id))?;

    eprintln!("image_id:       {}", stored.image_id);
    eprintln!("skeet_id:       {}", stored.skeet_id);
    eprintln!("zone:           {}", stored.zone);
    eprintln!("config_version: {}", stored.config_version);
    eprintln!("discovered_at:  {}", stored.discovered_at);
    eprintln!("original_at:    {}", stored.original_at);
    eprintln!("detected_text:  {:?}", stored.detected_text);
    eprintln!(
        "image_size:     {}x{}",
        stored.image.width(),
        stored.image.height()
    );
    eprintln!();

    let at_uri = stored.skeet_id.as_str();
    eprintln!("Fetching post thread for {at_uri} ...");

    let http = reqwest::Client::new();
    let json = skeet_finder::metadata::fetch_post_thread(&http, at_uri).await?;
    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}
