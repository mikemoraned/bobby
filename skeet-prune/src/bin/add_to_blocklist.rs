#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use clap::Parser;
use shared::skeet_id::SkeetId;
use tracing::info;

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
    shared::tracing::init("info");

    let args = Args::parse();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let blocklist_dir = root.join("blocklist");

    let skeet_id: SkeetId = args.at_uri.parse()?;
    let rkey = skeet_id.rkey();

    info!(at_uri = %skeet_id, "fetching post thread");
    let http = reqwest::Client::new();
    let json = skeet_prune::metadata::fetch_post_thread(&http, &skeet_id).await?;

    let json_path = blocklist_dir.join(format!("{rkey}.json"));
    let pretty = serde_json::to_string_pretty(&json)?;
    std::fs::write(&json_path, &pretty)?;
    info!(path = %json_path.display(), "saved JSON");

    let toml_path = blocklist_dir.join("blocklist.toml");
    let mut config = shared::BlocklistConfig::from_file(&toml_path)
        .unwrap_or_default();

    let entry = shared::BlockedEntry {
        skeet_id,
        reason: args.reason,
    };

    if config.add(entry) {
        config.save(&toml_path)?;
        info!(path = %toml_path.display(), "added to blocklist");
    } else {
        info!("URI already in blocklist, skipping config update");
    }

    Ok(())
}
