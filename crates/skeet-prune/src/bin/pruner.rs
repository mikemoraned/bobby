#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use shared::{PruneConfig, RejectionCategory};
use skeet_prune::{
    ChannelMonitors, ImageMessage, MetaMessage, PipelineCounters, SkeetCandidate, StatsMessage,
};
use skeet_store::StoreArgs;
use tokio_util::sync::CancellationToken;
use tracing::info;

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
    let channels = ChannelMonitors::new(firehose_tx.clone(), meta_tx.clone(), image_tx.clone());

    // Shared shutdown signal: any stage whose downstream closes cancels the
    // token, so every other stage unwinds through the same seam.
    let token = CancellationToken::new();

    let meta_http = http.clone();
    let firehose_counters = Arc::clone(&counters);
    let meta_counters = Arc::clone(&counters);
    let image_counters = Arc::clone(&counters);

    let firehose_token = token.clone();
    tokio::spawn(async move {
        skeet_prune::firehose_stage::run(firehose_tx, firehose_counters, firehose_token).await;
    });

    let meta_workers = args.meta_workers;
    let meta_token = token.clone();
    tokio::spawn(async move {
        skeet_prune::prune_meta_stage::run_workers(
            firehose_rx,
            meta_tx,
            meta_http,
            meta_counters,
            meta_workers,
            meta_token,
        )
        .await;
    });

    let image_workers = args.image_workers;
    let image_token = token.clone();
    let classify_config = skeet_prune::prune_image_stage::ClassifyConfig {
        http,
        prune_config,
        config_version,
    };
    tokio::spawn(async move {
        skeet_prune::prune_image_stage::run_workers(
            meta_rx,
            image_tx,
            classify_config,
            image_counters,
            image_workers,
            image_token,
        )
        .await;
    });

    let save_token = token.clone();
    let save_store = Arc::clone(&store);
    tokio::spawn(async move {
        skeet_prune::save_stage::run(&image_rx, save_store.as_ref(), stats_tx, save_token).await;
    });

    let log_interval = std::time::Duration::from_secs(args.status_interval_secs);
    let flush_interval = std::time::Duration::from_secs(args.statistics_flush_secs);
    skeet_prune::content_statistics_stage::run(
        &stats_rx,
        store.as_ref(),
        counters,
        channels,
        log_interval,
        flush_interval,
        token,
    )
    .await;

    Ok(())
}
