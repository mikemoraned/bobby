use std::path::PathBuf;
use std::slice;

use chrono::Utc;
use clap::Parser;
use eval::update_prices::{MODELS_DEV_URL, extract_prices};
use eval::{PricesRegistry, Snapshot, SnapshotId};
use shared::refine_model::Label;
use tracing::info;

#[derive(Parser)]
#[command(
    about = "Fetch OpenAI pricing from models.dev and append a snapshot to eval/prices.toml"
)]
struct Args {
    /// Comma-separated list of model names to include
    #[arg(long, value_delimiter = ',')]
    models: Vec<String>,

    /// Path to the prices registry
    #[arg(long, default_value = "eval/prices.toml")]
    output: PathBuf,

    /// Label moved onto the freshly inserted snapshot
    #[arg(long, default_value = "current")]
    label: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    info!(url = MODELS_DEV_URL, models = ?args.models, "fetching pricing data");
    let json = reqwest::get(MODELS_DEV_URL)
        .await?
        .error_for_status()?
        .text()
        .await?;

    let prices = extract_prices(&json, &args.models)?;
    info!(count = prices.len(), "extracted prices");

    let mut registry = PricesRegistry::load_or_empty(&args.output)?;
    let label = Label::new(&args.label);
    let snapshot_id = SnapshotId::new(Utc::now());
    let snapshot = Snapshot {
        source_url: MODELS_DEV_URL.to_string(),
        note: None,
        prices,
    };
    registry.insert(snapshot_id, snapshot, slice::from_ref(&label))?;
    registry.save(&args.output)?;
    info!(
        path = %args.output.display(),
        %label,
        snapshot_id = %snapshot_id,
        "inserted snapshot and moved label"
    );

    Ok(())
}
