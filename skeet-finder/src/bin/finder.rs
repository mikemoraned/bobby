#![warn(clippy::all, clippy::nursery)]

use std::collections::HashMap;

use clap::Parser;
use face_detection::FaceDetector;
use shared::{ArchetypeConfig, Rejection};
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

    let receiver = skeet_finder::firehose::connect().await?;
    info!("firehose connected, listening for posts...");

    let mut post_count: u64 = 0;
    let mut image_post_count: u64 = 0;
    let mut saved_count: u64 = 0;
    let mut rejected_count: u64 = 0;
    let mut rejection_counts: HashMap<Rejection, u64> = HashMap::new();
    let mut last_log = std::time::Instant::now();
    let log_interval = std::time::Duration::from_secs(30);

    while let Ok(event) = receiver.recv_async().await {
        post_count += 1;

        let skeet_images = skeet_finder::firehose::extract_skeet_images(&event, &http).await;
        image_post_count += skeet_images.len() as u64;

        for skeet_image in skeet_images {
            match skeet_finder::classify_image(
                skeet_image,
                &detector,
                &text_detector,
                &archetype_config,
                &config_version,
            ) {
                Ok(record) => {
                    persistence::save(&store, &record, &mut saved_count).await;
                }
                Err(reasons) => {
                    rejected_count += 1;
                    for reason in &reasons {
                        *rejection_counts.entry(*reason).or_default() += 1;
                    }
                }
            }
        }

        if post_count == 1
            || post_count.is_multiple_of(100)
            || last_log.elapsed() >= log_interval
        {
            status::log_summary(
                post_count,
                image_post_count,
                saved_count,
                rejected_count,
                &rejection_counts,
            );
            last_log = std::time::Instant::now();
        }
    }

    warn!("firehose disconnected");
    Ok(())
}
