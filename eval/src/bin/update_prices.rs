use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use eval::update_prices::{MODELS_DEV_URL, extract_prices, render_prices_toml};
use tracing::info;

#[derive(Parser)]
#[command(about = "Fetch OpenAI pricing from models.dev and write eval/prices.toml with provenance")]
struct Args {
    /// Comma-separated list of model names to include
    #[arg(long, value_delimiter = ',')]
    models: Vec<String>,

    /// Output path for prices.toml
    #[arg(long, default_value = "eval/prices.toml")]
    output: PathBuf,
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

    let body = render_prices_toml(&prices)?;
    let now = Utc::now().to_rfc3339();
    let content =
        format!("# Source: {MODELS_DEV_URL}\n# Fetched: {now}\n\n{body}");
    std::fs::write(&args.output, content)?;
    info!(path = %args.output.display(), "wrote prices.toml");

    Ok(())
}
