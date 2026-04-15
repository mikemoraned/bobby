#![warn(clippy::all, clippy::nursery)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use clap::Parser;
use tracing::info;

#[derive(Parser)]
struct Args {
    /// How long to collect firehose events in seconds
    #[arg(long, default_value_t = 300)]
    duration_secs: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let _guard = shared::tracing::init_with_file(
        "bench_firehose=info,skeet_prune=info,shared=info",
        "bench-firehose.log",
        shared::tracing::TokioConsoleSupport::Disabled,
    );

    let run_duration = Duration::from_secs(args.duration_secs);
    let recv_timeout = Duration::from_secs(5);

    info!(
        duration_secs = args.duration_secs,
        "phase 1: collecting firehose events"
    );

    let receiver = skeet_prune::firehose::connect().await?;
    info!("firehose connected, collecting events");

    let started_at = Instant::now();
    let mut total_events: u64 = 0;
    let mut total_candidates: u64 = 0;
    let mut image_urls: Vec<String> = Vec::new();
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
                    image_urls.extend(candidate.image_urls);
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
            Err(_) if logged_first => {
                // Transient gap after events were flowing — keep waiting for duration
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
    let total_images = image_urls.len() as u64;
    let images_per_sec = total_images as f64 / elapsed;
    let candidate_pct = if total_events > 0 {
        (total_candidates as f64 / total_events as f64) * 100.0
    } else {
        0.0
    };

    info!("=== phase 1: firehose results ===");
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

    // Phase 2: fetch each collected image URL one at a time, measuring latency and bytes
    info!(
        total_images,
        "phase 2: fetching images sequentially"
    );

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let mut results_by_status: HashMap<u16, StatusStats> = HashMap::new();

    for (i, url) in image_urls.iter().enumerate() {
        let start = Instant::now();
        let outcome = http.get(url).send().await;

        match outcome {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let bytes = resp.bytes().await.map_or(0, |b| b.len() as u64);
                let latency = start.elapsed();
                let stats = results_by_status.entry(status).or_default();
                stats.count += 1;
                stats.total_bytes += bytes;
                stats.total_latency += latency;
                if latency < stats.min_latency {
                    stats.min_latency = latency;
                }
                if latency > stats.max_latency {
                    stats.max_latency = latency;
                }
            }
            Err(_) => {
                let latency = start.elapsed();
                let stats = results_by_status.entry(0).or_default();
                stats.count += 1;
                stats.total_latency += latency;
            }
        }

        if (i + 1).is_multiple_of(500) {
            info!(fetched = i + 1, remaining = total_images as usize - (i + 1), "progress");
        }
    }

    info!("=== phase 2: image fetch results ===");
    let mut statuses: Vec<_> = results_by_status.iter().collect();
    statuses.sort_by_key(|(s, _)| *s);
    for &(status, stats) in &statuses {
        let avg_latency_ms = stats.total_latency.as_secs_f64() * 1000.0 / stats.count as f64;
        let avg_bytes = if stats.total_bytes > 0 {
            stats.total_bytes as f64 / stats.count as f64
        } else {
            0.0
        };
        let label = if *status == 0 { "error".to_string() } else { status.to_string() };
        info!(
            status = label,
            count = stats.count,
            avg_latency_ms = format_args!("{avg_latency_ms:.1}"),
            min_latency_ms = format_args!("{:.1}", stats.min_latency.as_secs_f64() * 1000.0),
            max_latency_ms = format_args!("{:.1}", stats.max_latency.as_secs_f64() * 1000.0),
            avg_bytes = format_args!("{avg_bytes:.0}"),
            total_bytes = stats.total_bytes,
            "by status"
        );
    }

    Ok(())
}

struct StatusStats {
    count: u64,
    total_bytes: u64,
    total_latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
}

impl Default for StatusStats {
    fn default() -> Self {
        Self {
            count: 0,
            total_bytes: 0,
            total_latency: Duration::ZERO,
            min_latency: Duration::MAX,
            max_latency: Duration::ZERO,
        }
    }
}
