#![warn(clippy::all, clippy::nursery)]

use std::time::{Duration, Instant};

use clap::Parser;
use tokio::sync::mpsc;
use tracing::info;

use skeet_prune::firehose::SkeetCandidate;

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

    let (tx, mut rx) = mpsc::channel::<SkeetCandidate>(10_000);

    tokio::spawn(async move {
        skeet_prune::firehose_stage::run(tx).await;
    });

    let started_at = Instant::now();
    let mut total_candidates: u64 = 0;
    let mut total_images: u64 = 0;
    let mut logged_first = false;

    loop {
        let remaining = run_duration.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            break;
        }

        let timeout = remaining.min(recv_timeout);
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Some(candidate)) => {
                let image_count = candidate.image_urls.len() as u64;
                total_candidates += 1;
                total_images += image_count;

                if !logged_first {
                    info!("first candidate received, collecting for {duration_secs}s", duration_secs = args.duration_secs);
                    logged_first = true;
                }
            }
            Ok(None) => {
                info!("firehose channel closed unexpectedly");
                break;
            }
            Err(_) if started_at.elapsed() >= run_duration => {
                break;
            }
            Err(_) => {
                info!("no candidate received in {recv_timeout:?}, exiting");
                break;
            }
        }
    }

    let elapsed = started_at.elapsed().as_secs_f64();
    let candidates_per_sec = total_candidates as f64 / elapsed;
    let images_per_sec = total_images as f64 / elapsed;

    info!("=== firehose benchmark results ===");
    info!(
        elapsed_secs = format_args!("{elapsed:.1}"),
        total_candidates,
        total_images,
        "totals"
    );
    info!(
        candidates_per_sec = format_args!("{candidates_per_sec:.1}"),
        images_per_sec = format_args!("{images_per_sec:.1}"),
        "throughput"
    );

    Ok(())
}
