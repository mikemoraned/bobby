#![warn(clippy::all, clippy::nursery)]

use std::str::FromStr;

use chrono::{DateTime, Utc};
use clap::Parser;
use cloudflare_exporter::{
    cloudflare,
    metrics::SyncMetrics,
    prom,
    types::{AccountTag, ApiToken},
};
use tracing::info;

#[derive(Parser)]
#[command(
    about = "Sync Cloudflare R2 operations metrics to Grafana Cloud via Prometheus remote_write"
)]
struct Args {
    /// Cloudflare API token (Account Analytics: Read scope)
    #[arg(long, env = "BOBBY_CLOUDFLARE_API_TOKEN", value_parser = ApiToken::from_str)]
    api_token: ApiToken,

    /// Cloudflare account tag (32-char hex account ID)
    #[arg(long, env = "BOBBY_CLOUDFLARE_ACCOUNT_TAG", value_parser = AccountTag::from_str)]
    account_tag: AccountTag,

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

async fn sync(args: &Args) -> Result<u64, Box<dyn std::error::Error>> {
    let now = Utc::now();
    let from = args
        .from
        .unwrap_or_else(|| now - chrono::Duration::minutes(6));
    let to = args
        .to
        .unwrap_or_else(|| now - chrono::Duration::minutes(5));

    info!(%from, %to, "syncing Cloudflare R2 operations metrics");

    let client = reqwest::Client::new();
    let mut total_datapoints: u64 = 0;
    for (window_from, window_to) in cloudflare::one_minute_windows(from, to) {
        let timestamp_ms = (window_from.timestamp_millis() + window_to.timestamp_millis()) / 2;

        let fetched = cloudflare::fetch_r2_operations(
            &client,
            &args.api_token,
            &args.account_tag,
            window_from,
            window_to,
        )
        .await?;

        let datapoints = fetched.operations.len() as u64;
        total_datapoints += datapoints;

        info!(
            %window_from,
            %window_to,
            operations = fetched.operations.len(),
            "fetched and pushing"
        );

        prom::push_operations(
            &client,
            &args.prom_endpoint,
            &args.prom_auth,
            &fetched,
            timestamp_ms,
        )
        .await?;
    }

    Ok(total_datapoints)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "cloudflare-exporter");
    info!(
        git_hash = env!("BUILD_GIT_HASH"),
        "cloudflare-exporter sync starting"
    );

    let args = Args::parse();

    let meter = opentelemetry::global::meter("cloudflare-exporter");
    let sync_metrics = SyncMetrics::new(&meter);

    match sync(&args).await {
        Ok(datapoints) => {
            info!(datapoints, "all windows pushed via Prometheus remote_write");
            sync_metrics.record_success(datapoints);
            Ok(())
        }
        Err(e) => {
            sync_metrics.record_failure();
            Err(e)
        }
    }
}
