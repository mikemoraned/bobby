#![warn(clippy::all, clippy::nursery)]

use std::str::FromStr;

use chrono::Utc;
use clap::Parser;
use cloudflare_exporter::{
    metrics::SyncMetrics,
    prom,
    r2_rest,
    types::{AccountTag, ApiToken},
};
use tracing::info;

#[derive(Parser)]
#[command(about = "Sync Cloudflare R2 storage gauges to Grafana Cloud via Prometheus remote_write")]
struct Args {
    /// Cloudflare API token (Account R2 Storage: Read scope)
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
}

async fn sync(args: &Args) -> Result<u64, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let buckets = r2_rest::list_buckets(&client, &args.api_token, &args.account_tag).await?;
    info!(bucket_count = buckets.len(), "fetched bucket list");

    let mut usages = Vec::with_capacity(buckets.len());
    for bucket in buckets {
        let usage =
            r2_rest::fetch_bucket_usage(&client, &args.api_token, &args.account_tag, &bucket.name)
                .await?;
        info!(
            bucket = %bucket.name,
            payload_size = usage.payload_size,
            object_count = usage.object_count,
            "fetched bucket usage"
        );
        usages.push((bucket, usage));
    }

    let timestamp_ms = Utc::now().timestamp_millis();
    let datapoints = (usages.len() * 2) as u64;
    prom::push_storage(
        &client,
        &args.prom_endpoint,
        &args.prom_auth,
        &usages,
        timestamp_ms,
    )
    .await?;

    Ok(datapoints)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "cloudflare-exporter");
    info!(
        git_hash = env!("BUILD_GIT_HASH"),
        "cloudflare-exporter sync_storage starting"
    );

    let args = Args::parse();

    let meter = opentelemetry::global::meter("cloudflare-exporter");
    let sync_metrics = SyncMetrics::new(&meter);

    match sync(&args).await {
        Ok(datapoints) => {
            info!(datapoints, "all bucket usages pushed via Prometheus remote_write");
            sync_metrics.record_success(datapoints);
            Ok(())
        }
        Err(e) => {
            sync_metrics.record_failure();
            Err(e)
        }
    }
}
