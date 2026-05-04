#![warn(clippy::all, clippy::nursery)]

use chrono::{DateTime, Utc};
use clap::Parser;
use cloudflare_exporter::{cloudflare, prom};
use tracing::info;

#[derive(Parser)]
#[command(about = "Sync Cloudflare R2 metrics to Grafana Cloud via Prometheus remote_write")]
struct Args {
    /// Cloudflare API token (Account Analytics: Read scope)
    #[arg(long, env = "BOBBY_CLOUDFLARE_API_TOKEN")]
    api_token: String,

    /// Cloudflare account tag (32-char hex account ID)
    #[arg(long, env = "BOBBY_CLOUDFLARE_ACCOUNT_TAG")]
    account_tag: String,

    /// Prometheus remote_write endpoint URL
    #[arg(long, env = "BOBBY_PROM_ENDPOINT")]
    prom_endpoint: String,

    /// Basic auth credentials (instance_id:api_key)
    #[arg(long, env = "BOBBY_PROM_AUTH")]
    prom_auth: String,

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
    info!(
        git_hash = env!("BUILD_GIT_HASH"),
        "cloudflare-exporter sync starting"
    );

    let args = Args::parse();

    let now = Utc::now();
    let from = args.from.unwrap_or_else(|| now - chrono::Duration::minutes(6));
    let to = args.to.unwrap_or_else(|| now - chrono::Duration::minutes(5));

    info!(%from, %to, "syncing Cloudflare R2 metrics");

    let client = reqwest::Client::new();
    for (window_from, window_to) in cloudflare::one_minute_windows(from, to) {
        let timestamp_ms = (window_from.timestamp_millis() + window_to.timestamp_millis()) / 2;

        let metrics = cloudflare::fetch_r2_metrics(
            &client,
            &args.api_token,
            &args.account_tag,
            window_from,
            window_to,
        )
        .await?;

        info!(
            %window_from,
            %window_to,
            operations = metrics.operations.len(),
            storage_entries = metrics.storage.len(),
            "fetched and pushing"
        );

        prom::push(
            &client,
            &args.prom_endpoint,
            &args.prom_auth,
            &metrics,
            timestamp_ms,
        )
        .await?;
    }

    info!("all windows pushed via Prometheus remote_write");
    Ok(())
}
