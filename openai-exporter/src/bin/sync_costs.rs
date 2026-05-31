#![warn(clippy::all, clippy::nursery)]

use chrono::{DateTime, Datelike, DurationRound, TimeZone, Utc};
use clap::Parser;
use openai_exporter::{metrics::SyncMetrics, openai, prom};
use tracing::info;

#[derive(Parser)]
#[command(about = "Sync OpenAI daily cost data to Grafana Cloud via Prometheus remote_write")]
struct Args {
    /// OpenAI admin API key (Usage API read scope)
    #[arg(long, env = "BOBBY_OPENAI_ADMIN_KEY")]
    openai_admin_key: String,

    /// Prometheus remote_write endpoint URL
    #[arg(long, env = "BOBBY_PROM_ENDPOINT")]
    prom_endpoint: String,

    /// Basic auth credentials (instance_id:api_key)
    #[arg(long, env = "BOBBY_PROM_AUTH")]
    prom_auth: String,

    /// Window start (RFC 3339); defaults to start of yesterday UTC
    #[arg(long)]
    from: Option<DateTime<Utc>>,

    /// Window end (RFC 3339); defaults to start of today UTC
    #[arg(long)]
    to: Option<DateTime<Utc>>,
}

fn start_of_day(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(dt.date_naive().year(), dt.date_naive().month(), dt.date_naive().day(), 0, 0, 0)
        .single()
        .unwrap_or(dt)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "openai-exporter");
    info!(
        git_hash = env!("BUILD_GIT_HASH"),
        "openai-exporter sync_costs starting"
    );

    let args = Args::parse();

    let meter = opentelemetry::global::meter("openai-exporter");
    let sync_metrics = SyncMetrics::new(&meter);

    match sync(&args).await {
        Ok(entries) => {
            info!(entries, "pushed to Prometheus remote_write");
            sync_metrics.record_success(entries);
            Ok(())
        }
        Err(e) => {
            sync_metrics.record_failure();
            Err(e)
        }
    }
}

async fn sync(args: &Args) -> Result<u64, Box<dyn std::error::Error>> {
    let today = start_of_day(Utc::now());
    let yesterday = start_of_day(today - chrono::Duration::days(1));

    let from = args.from.unwrap_or(yesterday);
    let to = args.to.unwrap_or(today);

    info!(%from, %to, "fetching OpenAI costs");

    let client = reqwest::Client::new();
    let entries = openai::fetch_costs(&client, &args.openai_admin_key, from, to).await?;

    info!(entries = entries.len(), "fetched cost entries");

    if entries.is_empty() {
        info!("no cost entries for window, nothing to push");
        return Ok(0);
    }

    // Floor to start of current hour so a run at 00:05 stamps 00:00,
    // staying within Mimir's past-grace-period regardless of cron jitter.
    let start_of_hour = Utc::now().duration_trunc(chrono::Duration::hours(1))?;
    let timestamp_ms = start_of_hour.timestamp_millis();

    prom::push(&client, &args.prom_endpoint, &args.prom_auth, &entries, timestamp_ms).await?;

    Ok(entries.len() as u64)
}
