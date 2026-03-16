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
    let json = skeet_finder::metadata::fetch_post_thread(&http, &args.at_uri).await?;

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
