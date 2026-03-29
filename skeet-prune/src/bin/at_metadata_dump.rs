#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(about = "Fetch and dump the Bluesky post thread JSON for any at:// URI")]
struct Args {
    #[arg(long)]
    at_uri: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();
    let skeet_id: shared::skeet_id::SkeetId = args.at_uri.parse()?;

    info!(at_uri = %skeet_id, "fetching post thread");

    let http = reqwest::Client::new();
    let json = skeet_prune::metadata::fetch_post_thread(&http, &skeet_id).await?;
    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}
