#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use shared::{PruneConfig, RejectionCategory};
use skeet_prune::{
    ChannelMonitors, ImageMessage, MetaMessage, PipelineCounters, SkeetCandidate, StatsMessage,
};
use skeet_store::StoreArgs;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to prune.toml config file
    #[arg(long)]
    config_path: PathBuf,

    /// Status log interval in seconds (default: 30)
    #[arg(long, default_value = "30")]
    status_interval_secs: u64,

    /// How often, in seconds, buffered prune statistics are written to the store
    /// as one batch (default: 600). Larger values cut store write churn at the
    /// cost of a longer window of unwritten stats on crash.
    #[arg(long, default_value = "600")]
    statistics_flush_secs: u64,

    /// Number of parallel meta stage workers (default: 4). The meta stage is
    /// network-I/O-bound (one `getPostThread` round-trip per candidate), so a
    /// small pool keeps it from capping the pipeline at the firehose rate.
    #[arg(long, default_value = "4")]
    meta_workers: usize,

    /// Number of parallel image stage workers (default: 2)
    #[arg(long, default_value = "2")]
    image_workers: usize,

    /// Seconds to let the pipeline drain into the store after a shutdown signal
    /// before forcing exit (default: 25). Keep below the k8s
    /// terminationGracePeriodSeconds so the drain finishes ahead of SIGKILL.
    #[arg(long, default_value = "25")]
    drain_timeout_secs: u64,

    /// Permit writing to a remote, shared object store (e.g. R2). Off by
    /// default: the pruner is the one writer to the shared `images_vN` table
    /// keyed by content hash *without* a per-owner discriminator, so a staging
    /// pruner running at the same table version would overwrite production's
    /// rows in place. Iterate the pruner offline against a local `file://`
    /// store; only the promoted pruner sets this flag (in production's
    /// deployment manifest).
    #[arg(long, default_value = "false")]
    allow_shared_store_write: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // `jetstream_oxide` logs the underlying WebSocket disconnect reason (and the
    // server close code) via the `log` crate, bridged into tracing; surface it at
    // `warn` so reconnect causes land in `pruner.log` without needing `RUST_LOG`.
    let _guard = shared::tracing::init_with_file(
        env!("CARGO_CRATE_NAME"),
        "skeet_prune=info,shared=info,skeet_store=info,lance_io=warn,object_store=warn,jetstream_oxide=warn",
        "pruner.log",
    );

    info!(git_hash = env!("BUILD_GIT_HASH"), "pruner starting");

    if args.store.is_remote() && !args.allow_shared_store_write {
        return Err(format!(
            "refusing to write remote shared store {:?}: the pruner overwrites \
             shared images rows in place; iterate offline against a local store, \
             or pass --allow-shared-store-write for the promoted production pruner",
            args.store.store_path
        )
        .into());
    }

    let http = reqwest::Client::new();

    let prune_config = PruneConfig::from_file(&args.config_path, None)?;
    let config_version = prune_config.version();

    info!(
        config_version = %config_version,
        categories = ?prune_config.categories(),
        "prune config loaded"
    );

    // Early sanity check: verify all required models can be loaded before
    // starting the pipeline, so we fail fast with clear errors.
    if prune_config.is_category_enabled(RejectionCategory::Text) {
        info!("validating text detection models");
        text_detection::TextDetector::from_bundled_models()?;
        info!("text detection models validated");
    }

    let store = args.store.open_store("pruner").await?;
    store.validate().await?;
    info!("storage validation passed");
    // Shared between the save stage (writes images) and the stats stage (records
    // per-interval PruneStats).
    let store = Arc::new(store);

    // Pipeline: firehose → meta prune → image prune → save → stats. The
    // firehose→meta and meta→image channels are MPMC so each stage's worker pool
    // shares one input; the image→save and save→stats channels each have a
    // single consumer but use the same channel type.
    let (firehose_tx, firehose_rx) = async_channel::bounded::<SkeetCandidate>(16);
    let (meta_tx, meta_rx) = async_channel::bounded::<MetaMessage>(16);
    let (image_tx, image_rx) = async_channel::bounded::<ImageMessage>(100);
    let (stats_tx, stats_rx) = async_channel::bounded::<StatsMessage>(100);

    let counters = Arc::new(PipelineCounters::default());
    let channels = ChannelMonitors::new(&firehose_tx, &meta_tx, &image_tx);

    // Two shutdown signals with distinct intent:
    // - `abort` (reactive): a stage whose downstream closes cancels it, so every
    //   other stage unwinds at once through the same seam — in-flight work is
    //   dropped because there's nowhere for it to go.
    // - `drain` (deliberate): a shutdown signal trips this to stop *only* the
    //   firehose source. The bounded channels then close-cascade so buffered and
    //   in-flight items finish into the idempotent store before exit.
    let abort = CancellationToken::new();
    let drain = CancellationToken::new();

    let signal_abort = abort.clone();
    let signal_drain = drain.clone();
    tokio::spawn(async move {
        match install_shutdown_signals() {
            Ok((mut sigterm, mut sigint)) => {
                wait_either(&mut sigterm, &mut sigint).await;
                info!("shutdown signal received, draining pipeline (signal again to abort)");
                signal_drain.cancel();
                wait_either(&mut sigterm, &mut sigint).await;
                warn!("second shutdown signal received, aborting drain");
                signal_abort.cancel();
            }
            Err(e) => {
                warn!(error = %e, "failed to install signal handlers; SIGTERM will fall back to SIGKILL");
            }
        }
    });

    let meta_http = http.clone();
    let firehose_counters = Arc::clone(&counters);
    let meta_counters = Arc::clone(&counters);
    let image_counters = Arc::clone(&counters);

    // Supervise every stage so shutdown awaits their completion, not just the
    // sink's: the drain finishes only once the sink stage returns, which happens
    // after everything upstream has drained through it into the store.
    let mut stages = JoinSet::new();

    let firehose_abort = abort.clone();
    let firehose_drain = drain.clone();
    stages.spawn(async move {
        skeet_prune::firehose_stage::run(
            firehose_tx,
            firehose_counters,
            firehose_abort,
            firehose_drain,
        )
        .await;
    });

    let meta_workers = args.meta_workers;
    let meta_abort = abort.clone();
    stages.spawn(async move {
        skeet_prune::prune_meta_stage::run_workers(
            firehose_rx,
            meta_tx,
            meta_http,
            meta_counters,
            meta_workers,
            meta_abort,
        )
        .await;
    });

    let image_workers = args.image_workers;
    let image_abort = abort.clone();
    let classify_config = skeet_prune::prune_image_stage::ClassifyConfig {
        http,
        prune_config,
        config_version,
    };
    stages.spawn(async move {
        skeet_prune::prune_image_stage::run_workers(
            meta_rx,
            image_tx,
            classify_config,
            image_counters,
            image_workers,
            image_abort,
        )
        .await;
    });

    let save_abort = abort.clone();
    let save_store = Arc::clone(&store);
    stages.spawn(async move {
        skeet_prune::save_stage::run(&image_rx, save_store.as_ref(), stats_tx, save_abort).await;
    });

    let log_interval = std::time::Duration::from_secs(args.status_interval_secs);
    let flush_interval = std::time::Duration::from_secs(args.statistics_flush_secs);
    let stats_store = Arc::clone(&store);
    let stats_abort = abort.clone();
    stages.spawn(async move {
        skeet_prune::content_statistics_stage::run(
            &stats_rx,
            stats_store.as_ref(),
            counters,
            channels,
            log_interval,
            flush_interval,
            stats_abort,
        )
        .await;
    });

    let drain_timeout = std::time::Duration::from_secs(args.drain_timeout_secs);
    tokio::select! {
        () = await_stages(&mut stages) => info!("pipeline stages completed, exiting"),
        () = drain_deadline(&drain, drain_timeout) => {
            warn!(
                timeout_secs = args.drain_timeout_secs,
                "drain deadline exceeded; exiting with items possibly still in flight"
            );
        }
    }

    Ok(())
}

/// Install the process-termination signal streams: SIGTERM (k8s redeploy, sent
/// before the grace-period SIGKILL) and SIGINT (Ctrl-C during local iteration).
fn install_shutdown_signals() -> std::io::Result<(Signal, Signal)> {
    Ok((signal(SignalKind::terminate())?, signal(SignalKind::interrupt())?))
}

/// Resolve when either signal stream next fires.
async fn wait_either(sigterm: &mut Signal, sigint: &mut Signal) {
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = sigint.recv() => {}
    }
}

/// Await every supervised stage to completion, logging any that failed to join.
async fn await_stages(stages: &mut JoinSet<()>) {
    while let Some(res) = stages.join_next().await {
        if let Err(e) = res {
            warn!(error = %e, "pipeline stage task failed");
        }
    }
}

/// Bound the post-signal drain: resolves `timeout` after `drain` is tripped, and
/// never before. Normal running and reactive abort leave `drain` untripped, so
/// this future stays pending and `main` waits on the stages with no timer —
/// only a deliberate drain is deadline-bounded.
async fn drain_deadline(drain: &CancellationToken, timeout: std::time::Duration) {
    drain.cancelled().await;
    tokio::time::sleep(timeout).await;
}
