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

    info!(at_uri = %args.at_uri, "fetching post thread");

    let http = reqwest::Client::new();
    let json = skeet_finder::metadata::fetch_post_thread(&http, &args.at_uri).await?;
    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}
