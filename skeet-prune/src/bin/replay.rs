#![warn(clippy::all, clippy::nursery)]

use std::time::Duration;

use clap::Parser;
use face_detection::FaceDetector;
use shared::{Classification, Percentage, PruneConfig};
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let _guard = shared::tracing::init_with_file_and_stderr(
        "skeet_prune=info,shared=info,skeet_store=info,lance_io=warn,object_store=warn",
        "replay.log",
        shared::tracing::TokioConsoleSupport::Disabled,
    );

    let detector = FaceDetector::from_bundled_weights();
    let text_detector = text_detection::TextDetector::from_bundled_models();

    let prune_config = PruneConfig::from_file(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/prune.toml"),
    )?;
    let config_version = prune_config.version();
    info!(config_version = %config_version, "models loaded");

    let store = args.store.open_store().await?;

    info!("loading all images from store...");
    let images = store.list_all().await?;
    let total = images.len();
    info!(total, "images loaded, starting replay");

    let mut status = skeet_prune::status::Status::new(Duration::from_secs(10), 100);

    for (i, stored) in images.into_iter().enumerate() {
        let image = &stored.image;
        let skin_mask = skin_detection::detect_skin(image);
        let text_result = text_detector.detect(image);
        let text_area_pct =
            Percentage::new(text_result.text_area_pct(image.width(), image.height()));
        let faces = detector.detect(image);

        let classification =
            skeet_prune::classify(&faces, image, &skin_mask, text_area_pct, &prune_config);

        status.record_post(1);
        match classification {
            Classification::Accepted(_) => status.record_saved(),
            Classification::Rejected(reasons) => status.record_rejected(&reasons),
        }

        if (i + 1) % 50 == 0 {
            info!("replayed {}/{total}", i + 1);
        }
    }

    info!("replay complete");
    status.log_summary();

    Ok(())
}
