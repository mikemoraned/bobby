#![warn(clippy::all, clippy::nursery)]

use clap::Parser;

#[derive(Parser)]
#[command(about = "Fetch and dump the Bluesky post thread JSON for any at:// URI")]
struct Args {
    #[arg(long)]
    at_uri: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    eprintln!("Fetching post thread for {} ...", args.at_uri);

    let http = reqwest::Client::new();
    let json = skeet_finder::metadata::fetch_post_thread(&http, &args.at_uri).await?;
    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}
