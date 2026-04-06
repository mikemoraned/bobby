#![warn(clippy::all, clippy::nursery)]

use std::time::{Duration, Instant};

use clap::Parser;
use tracing::info;

#[derive(Parser)]
struct Args {
    /// How long to run the benchmark in seconds
    #[arg(long, default_value_t = 300)]
    duration_secs: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let _guard = shared::tracing::init_with_file_and_stderr(
        "bench_firehose=info,skeet_prune=info,shared=info",
        "bench-firehose.log",
        shared::tracing::TokioConsoleSupport::Disabled,
    );

    let run_duration = Duration::from_secs(args.duration_secs);
    let recv_timeout = Duration::from_secs(5);

    info!(
        duration_secs = args.duration_secs,
        "starting firehose benchmark"
    );

    let receiver = skeet_prune::firehose::connect().await?;
    info!("firehose connected, collecting events");

    let started_at = Instant::now();
    let mut total_events: u64 = 0;
    let mut total_candidates: u64 = 0;
    let mut total_images: u64 = 0;
    let mut logged_first = false;

    loop {
        let remaining = run_duration.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            break;
        }

        let timeout = remaining.min(recv_timeout);
        match tokio::time::timeout(timeout, receiver.recv_async()).await {
            Ok(Ok(event)) => {
                total_events += 1;

                if let Some(candidate) =
                    skeet_prune::firehose::extract_skeet_candidate(&event)
                {
                    total_candidates += 1;
                    total_images += candidate.image_urls.len() as u64;
                }

                if !logged_first {
                    info!(
                        "first event received, collecting for {duration_secs}s",
                        duration_secs = args.duration_secs
                    );
                    logged_first = true;
                }
            }
            Ok(Err(_)) => {
                info!("firehose channel closed unexpectedly");
                break;
            }
            Err(_) if started_at.elapsed() >= run_duration => {
                break;
            }
            Err(_) => {
                info!("no event received in {recv_timeout:?}, exiting");
                break;
            }
        }
    }

    let elapsed = started_at.elapsed().as_secs_f64();
    let events_per_sec = total_events as f64 / elapsed;
    let candidates_per_sec = total_candidates as f64 / elapsed;
    let images_per_sec = total_images as f64 / elapsed;
    let candidate_pct = if total_events > 0 {
        (total_candidates as f64 / total_events as f64) * 100.0
    } else {
        0.0
    };

    info!("=== firehose benchmark results ===");
    info!(
        elapsed_secs = format_args!("{elapsed:.1}"),
        total_events,
        total_candidates,
        total_images,
        candidate_pct = format_args!("{candidate_pct:.1}%"),
        "totals"
    );
    info!(
        events_per_sec = format_args!("{events_per_sec:.1}"),
        candidates_per_sec = format_args!("{candidates_per_sec:.1}"),
        images_per_sec = format_args!("{images_per_sec:.1}"),
        "throughput"
    );

    Ok(())
}
