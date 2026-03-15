#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use clap::Parser;

#[derive(Parser)]
#[command(about = "Download a skeet's JSON and add it to the blocklist")]
struct Args {
    /// The at:// URI of the post to block
    at_uri: String,

    /// Reason for blocking (stored in the config)
    #[arg(long, default_value = "manual")]
    reason: String,
}

const BSKY_PUBLIC_API: &str = "https://public.api.bsky.app/xrpc";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let blocklist_dir = root.join("blocklist");

    // Extract rkey from the at:// URI for the filename
    let rkey = args
        .at_uri
        .rsplit('/')
        .next()
        .ok_or("invalid at:// URI: no rkey")?;

    // Download the post thread JSON
    eprintln!("Fetching {}", args.at_uri);
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{BSKY_PUBLIC_API}/app.bsky.feed.getPostThread"))
        .query(&[("uri", args.at_uri.as_str()), ("depth", "0"), ("parentHeight", "0")])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        eprintln!("API error ({status}): {body}");
        std::process::exit(1);
    }

    let json: serde_json::Value = resp.json().await?;

    // Save the JSON
    let json_path = blocklist_dir.join(format!("{rkey}.json"));
    let pretty = serde_json::to_string_pretty(&json)?;
    std::fs::write(&json_path, &pretty)?;
    eprintln!("Saved JSON to {}", json_path.display());

    // Add to blocklist.toml
    let toml_path = blocklist_dir.join("blocklist.toml");
    let mut config = shared::BlocklistConfig::from_file(&toml_path)
        .unwrap_or_default();

    let entry = shared::BlockedEntry {
        at_uri: args.at_uri.clone(),
        reason: args.reason,
    };

    if config.add(entry) {
        config.save(&toml_path)?;
        eprintln!("Added to {}", toml_path.display());
    } else {
        eprintln!("URI already in blocklist, skipping config update");
    }

    Ok(())
}
