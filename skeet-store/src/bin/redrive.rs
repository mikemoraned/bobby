#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::{ImageRecord, StoreArgs};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(about = "Redrive images from a local fallback store to a remote store")]
struct Args {
    /// Local fallback store path (source)
    #[arg(long)]
    source_store_path: String,

    /// Remote store (target)
    #[command(flatten)]
    target: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let source = skeet_store::SkeetStore::open(&args.source_store_path, vec![]).await?;
    info!(path = %args.source_store_path, "opened source (fallback) store");

    let target = args.target.open_store().await?;
    target.validate().await?;
    info!("target store validation passed");

    let images = source.list_all().await?;
    info!(count = images.len(), "found images in fallback store");

    let mut uploaded = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;

    for image in images {
        let image_id = image.summary.image_id.clone();

        match target.exists(&image_id).await {
            Ok(true) => {
                info!(image_id = %image_id, "already exists in target, skipping");
                skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                warn!(image_id = %image_id, error = %e, "failed to check existence, attempting upload");
            }
        }

        let record: ImageRecord = image.into();

        match target.add(&record).await {
            Ok(()) => {
                uploaded += 1;
                info!(image_id = %image_id, uploaded, "uploaded to target");
            }
            Err(e) => {
                failed += 1;
                error!(image_id = %image_id, error = %e, "failed to upload");
            }
        }
    }

    info!(uploaded, skipped, failed, "redrive complete");
    Ok(())
}
