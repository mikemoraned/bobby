#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use face_detection::FaceDetector;
use shared::ArchetypeConfig;
use skeet_finder::pipeline::FilterResult;
use skeet_store::StoreArgs;
use tokio::sync::mpsc;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

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
        "skeet_finder=info,shared=info,skeet_store=info",
        "finder.log",
        console,
    );

    let http = reqwest::Client::new();
    let detector = FaceDetector::from_bundled_weights();
    let text_detector = text_detection::TextDetector::from_bundled_models();

    let archetype_config = ArchetypeConfig::from_file(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/archetype.toml"),
    )?;
    let config_version = archetype_config.version();

    info!(config_version = %config_version, "face detection model loaded");

    let store = args.store.open_store().await?;
    store.validate().await?;
    info!("storage validation passed");

    let (tx, mut rx) = mpsc::channel::<FilterResult>(16);

    tokio::spawn(async move {
        skeet_finder::filter_stage::run(
            tx,
            http,
            detector,
            text_detector,
            archetype_config,
            config_version,
        )
        .await;
    });

    skeet_finder::save_stage::run(&mut rx, &store).await;

    Ok(())
}
