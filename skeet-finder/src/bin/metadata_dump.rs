#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;

use clap::Parser;
use skeet_store::{ImageId, SkeetStore};

#[derive(Parser)]
#[command(about = "Look up an image in the store and dump its Bluesky post metadata")]
struct Args {
    #[arg(long)]
    store_path: PathBuf,

    #[arg(long)]
    image_id: ImageId,
}

const BSKY_PUBLIC_API: &str = "https://public.api.bsky.app/xrpc";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let store = SkeetStore::open(&args.store_path).await?;
    let stored = store
        .get_by_id(&args.image_id)
        .await?
        .ok_or_else(|| format!("no image found with id {}", args.image_id))?;

    eprintln!("image_id:       {}", stored.image_id);
    eprintln!("skeet_id:       {}", stored.skeet_id);
    eprintln!("archetype:      {}", stored.archetype);
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
    let resp = http
        .get(format!("{BSKY_PUBLIC_API}/app.bsky.feed.getPostThread"))
        .query(&[("uri", at_uri), ("depth", "0"), ("parentHeight", "0")])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        eprintln!("API error ({status}): {body}");
        std::process::exit(1);
    }

    let json: serde_json::Value = resp.json().await?;
    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}
