#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;

use clap::Parser;
use face_detection::FaceDetector;
use shared::PruneConfig;
use skeet_prune::firehose::SkeetCandidate;
use skeet_prune::pipeline::{ImageResult, MetaResult};
use skeet_store::{SkeetStore, StoreArgs};
use tokio::sync::mpsc;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to prune.toml config file
    #[arg(long)]
    config_path: PathBuf,

    /// Local fallback store path for when remote saves fail
    #[arg(long)]
    fallback_local_store: Option<String>,

    /// Enable tokio-console on this port
    #[arg(long)]
    tokio_console_port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let console = args
        .tokio_console_port
        .map_or(shared::tracing::TokioConsoleSupport::Disabled, |port| {
            shared::tracing::TokioConsoleSupport::Enabled { port }
        });
    let _guard = shared::tracing::init_with_file_and_stderr(
        "skeet_prune=info,shared=info,skeet_store=info,lance_io=warn,object_store=warn",
        "pruner.log",
        console,
    );

    let http = reqwest::Client::new();
    let detector = FaceDetector::from_bundled_weights();

    let prune_config = PruneConfig::from_file(&args.config_path)?;
    let config_version = prune_config.version();

    info!(config_version = %config_version, "face detection model loaded");

    let store = args.store.open_store().await?;
    store.validate().await?;
    info!("storage validation passed");

    let fallback = match &args.fallback_local_store {
        Some(path) => {
            let fallback_store = SkeetStore::open(path, vec![], None).await?;
            info!(path = %path, "fallback local store opened");
            Some(fallback_store)
        }
        None => None,
    };

    // Pipeline: firehose → meta prune → image prune → save
    let (firehose_tx, mut firehose_rx) = mpsc::channel::<SkeetCandidate>(16);
    let (meta_tx, mut meta_rx) = mpsc::channel::<MetaResult>(16);
    let (image_tx, mut image_rx) = mpsc::channel::<ImageResult>(100);

    let meta_http = http.clone();

    tokio::spawn(async move {
        skeet_prune::firehose_stage::run(firehose_tx).await;
    });

    tokio::spawn(async move {
        skeet_prune::prune_meta_stage::run(&mut firehose_rx, meta_tx, meta_http).await;
    });

    tokio::spawn(async move {
        skeet_prune::prune_image_stage::run(
            &mut meta_rx,
            image_tx,
            http,
            detector,
            prune_config,
            config_version,
        )
        .await;
    });

    skeet_prune::save_stage::run(&mut image_rx, &store, fallback.as_ref()).await;

    Ok(())
}
