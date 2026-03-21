#![warn(clippy::all, clippy::nursery)]

mod persistence;
mod status;

use std::collections::HashMap;

use clap::Parser;
use face_detection::FaceDetector;
use shared::{ArchetypeConfig, Rejection};
use skeet_store::StoreArgs;
use tracing::{info, warn};

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("skeet_finder=info", "finder.log");

    let args = Args::parse();

    let store = args.store.open_store().await?;
    store.validate().await?;
    info!("storage validation passed");

    let http = reqwest::Client::new();
    let detector = FaceDetector::from_bundled_weights();
    let text_detector = text_detection::TextDetector::from_bundled_models();

    let archetype_config = ArchetypeConfig::from_file(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/archetype.toml"),
    )?;
    let config_version = archetype_config.version();

    info!(config_version = %config_version, "face detection model loaded");

    let receiver = skeet_finder::firehose::connect().await?;
    info!("firehose connected");

    let status = status::create_status();

    let mut post_count: u64 = 0;
    let mut image_post_count: u64 = 0;
    let mut saved_count: u64 = 0;
    let mut rejection_counts: HashMap<Rejection, u64> = HashMap::new();

    while let Ok(event) = receiver.recv_async().await {
        post_count += 1;

        let skeet_images = skeet_finder::firehose::extract_skeet_images(&event, &http).await;
        if skeet_images.is_empty() {
            if post_count.is_multiple_of(500) {
                status::update_status(
                    &status,
                    post_count,
                    image_post_count,
                    saved_count,
                    &rejection_counts,
                );
            }
            continue;
        }

        image_post_count += 1;

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
                    for reason in &reasons {
                        *rejection_counts.entry(*reason).or_default() += 1;
                    }
                }
            }
            status::update_status(
                &status,
                post_count,
                image_post_count,
                saved_count,
                &rejection_counts,
            );
        }
    }

    status.finish_with_message("jetstream connection closed");
    warn!("firehose disconnected");
    Ok(())
}
