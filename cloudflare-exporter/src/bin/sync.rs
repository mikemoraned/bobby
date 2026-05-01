#![warn(clippy::all, clippy::nursery)]

use chrono::{DateTime, Utc};
use clap::Parser;
use cloudflare_exporter::{cloudflare, otlp};
use tracing::info;

#[derive(Parser)]
#[command(about = "Sync Cloudflare R2 metrics to Grafana Cloud via OTLP")]
struct Args {
    /// Cloudflare API token (Account Analytics: Read scope)
    #[arg(long, env = "BOBBY_CLOUDFLARE_API_TOKEN")]
    api_token: String,

    /// Cloudflare account tag (32-char hex account ID)
    #[arg(long, env = "BOBBY_CLOUDFLARE_ACCOUNT_TAG")]
    account_tag: String,

    /// Window start (RFC 3339); defaults to now − 6min
    #[arg(long)]
    from: Option<DateTime<Utc>>,

    /// Window end (RFC 3339); defaults to now − 5min
    #[arg(long)]
    to: Option<DateTime<Utc>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "cloudflare-exporter");
    info!(git_hash = env!("BUILD_GIT_HASH"), "cloudflare-exporter sync starting");

    let args = Args::parse();

    let now = Utc::now();
    let from = args.from.unwrap_or_else(|| now - chrono::Duration::minutes(6));
    let to = args.to.unwrap_or_else(|| now - chrono::Duration::minutes(5));

    info!(%from, %to, "fetching Cloudflare R2 metrics");

    let client = reqwest::Client::new();
    let metrics =
        cloudflare::fetch_r2_metrics(&client, &args.api_token, &args.account_tag, from, to)
            .await?;

    info!(
        operations = metrics.operations.len(),
        storage_entries = metrics.storage.len(),
        "fetched Cloudflare R2 metrics"
    );

    let meter = opentelemetry::global::meter("cloudflare");
    let cf_metrics = otlp::CloudflareMetrics::new(meter);
    cf_metrics.record(&metrics);

    info!("metrics emitted");
    Ok(())
}
