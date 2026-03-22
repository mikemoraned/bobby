#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use face_detection::FaceDetector;
use shared::ArchetypeConfig;
use skeet_finder::{persistence, status};
use skeet_store::StoreArgs;
use tracing::{info, warn};

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

    let mut status = status::Status::new(std::time::Duration::from_secs(30), 100);
    let recv_timeout = std::time::Duration::from_secs(30);

    loop {
        let receiver = skeet_finder::firehose::connect().await?;
        info!("firehose connected, listening for posts...");

        loop {
            let event = match tokio::time::timeout(recv_timeout, receiver.recv_async()).await {
                Ok(Ok(event)) => event,
                Ok(Err(_)) => {
                    warn!("firehose channel closed");
                    break;
                }
                Err(_) => {
                    warn!("no message received in {recv_timeout:?}, reconnecting");
                    break;
                }
            };

            let skeet_images =
                skeet_finder::firehose::extract_skeet_images(&event, &http).await;
            let image_count = skeet_images.len() as u64;

            for skeet_image in skeet_images {
                match skeet_finder::classify_image(
                    skeet_image,
                    &detector,
                    &text_detector,
                    &archetype_config,
                    &config_version,
                ) {
                    Ok(record) => {
                        persistence::save(&store, &record, &mut status).await;
                    }
                    Err(reasons) => {
                        status.record_rejected(&reasons);
                    }
                }
            }

            status.record_post(image_count);
        }
    }
}
