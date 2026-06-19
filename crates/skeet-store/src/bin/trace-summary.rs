#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::tempo::{TempoClient, TempoError};
use tracing::info;

#[derive(Parser)]
#[command(about = "Fetch and summarise SkeetStore traces from Grafana Cloud Tempo")]
struct Args {
    /// Service name to search for
    #[arg(long, default_value = "skeet-live-refine")]
    service: String,

    /// Filter to traces containing a span with this name (e.g. list_unscored_image_ids)
    #[arg(long)]
    span: Option<String>,

    /// Number of traces to sample
    #[arg(long, default_value_t = 10)]
    sample: u32,

    /// How far back to look, in minutes
    #[arg(long, default_value_t = 60)]
    lookback_minutes: u64,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("missing env var {0}: set via op run --env-file bobby-grafana-otel.env")]
    MissingEnv(&'static str),
    #[error("Tempo error: {0}")]
    Tempo(#[from] TempoError),
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    shared::tracing::init("info");
    info!(git_hash = env!("BUILD_GIT_HASH"), "trace-summary starting");

    let args = Args::parse();

    let base_url = std::env::var("TEMPO_URL").map_err(|_| Error::MissingEnv("TEMPO_URL"))?;
    let user = std::env::var("TEMPO_USER").map_err(|_| Error::MissingEnv("TEMPO_USER"))?;
    let token = std::env::var("TEMPO_TOKEN").map_err(|_| Error::MissingEnv("TEMPO_TOKEN"))?;

    let client = TempoClient::new(base_url, user, token);
    let lookback_secs = args.lookback_minutes * 60;

    info!(
        service = %args.service,
        span = ?args.span,
        sample = args.sample,
        lookback_minutes = args.lookback_minutes,
        "searching for traces"
    );

    let traces = client
        .search(&args.service, args.span.as_deref(), args.sample, lookback_secs)
        .await?;

    if traces.is_empty() {
        println!(
            "No traces found for service '{}' in the last {} minutes.",
            args.service, args.lookback_minutes
        );
        return Ok(());
    }

    println!("Found {} trace(s). Fetching details...\n", traces.len());

    for trace_info in &traces {
        let trace = client.fetch_trace(trace_info).await?;
        let summary = skeet_store::trace_analysis::summarise(trace_info, &trace);
        print!("{summary}");
    }

    Ok(())
}
