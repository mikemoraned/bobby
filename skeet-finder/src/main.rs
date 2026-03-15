#![warn(clippy::all, clippy::nursery)]

mod classify_and_store;
mod firehose;

use std::path::PathBuf;

use clap::Parser;
use face_detection::{ArchetypeConfig, FaceDetector};
use skeet_store::SkeetStore;
use tracing::{info, warn};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store_path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skeet_finder=info".parse().expect("valid filter")),
        )
        .init();

    let args = Args::parse();

    let store = SkeetStore::open(&args.store_path).await?;
    let http = reqwest::Client::new();
    let detector = FaceDetector::from_bundled_weights();

    let archetype_config = ArchetypeConfig::from_file(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/archetype.toml"),
    )?;
    let config_version = archetype_config.version();

    info!(config_version = %config_version, "face detection model loaded");

    let receiver = firehose::connect().await?;

    info!("connected to jetstream, listening for posts...");

    let mut post_count: u64 = 0;
    let mut image_post_count: u64 = 0;
    let mut saved_count: u64 = 0;
    while let Ok(event) = receiver.recv_async().await {
        post_count += 1;

        let skeet_images = firehose::extract_skeet_images(&event, &http).await;
        if skeet_images.is_empty() {
            if post_count.is_multiple_of(500) {
                info!(
                    posts = post_count,
                    image_posts = image_post_count,
                    saved = saved_count,
                    "progress"
                );
            }
            continue;
        }

        image_post_count += 1;

        for skeet_image in skeet_images {
            if let Some(record) = classify_and_store::classify_image(
                skeet_image,
                &detector,
                &archetype_config,
                &config_version,
            ) {
                classify_and_store::save(&store, &record, &mut saved_count).await;
            }
        }
    }

    warn!("jetstream connection closed");
    Ok(())
}
