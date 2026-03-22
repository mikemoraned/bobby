#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::{ImageId, ImageRecord, SkeetStore, StoreArgs, StoredImage};
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

enum RedriveOutcome {
    Uploaded,
    VerifiedAndDeleted,
    ContentMismatch,
    Failed,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let source = SkeetStore::open(&args.source_store_path, vec![]).await?;
    info!(path = %args.source_store_path, "opened source (fallback) store");

    let target = args.target.open_store().await?;
    target.validate().await?;
    info!("target store validation passed");

    let images = source.list_all().await?;
    info!(count = images.len(), "found images in fallback store");

    let mut uploaded = 0u64;
    let mut verified_and_deleted = 0u64;
    let mut content_mismatch = 0u64;
    let mut failed = 0u64;

    for image in images {
        let outcome = redrive_image(image, &source, &target).await;
        match outcome {
            RedriveOutcome::Uploaded => uploaded += 1,
            RedriveOutcome::VerifiedAndDeleted => verified_and_deleted += 1,
            RedriveOutcome::ContentMismatch => content_mismatch += 1,
            RedriveOutcome::Failed => failed += 1,
        }
    }

    info!(
        uploaded,
        verified_and_deleted,
        content_mismatch,
        failed,
        "redrive complete"
    );
    Ok(())
}

async fn redrive_image(
    image: StoredImage,
    source: &SkeetStore,
    target: &SkeetStore,
) -> RedriveOutcome {
    let image_id = image.summary.image_id.clone();

    match target.exists(&image_id).await {
        Ok(true) => verify_and_delete(image, &image_id, source, target).await,
        Ok(false) => upload(image, &image_id, target).await,
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to check existence");
            RedriveOutcome::Failed
        }
    }
}

async fn verify_and_delete(
    local_image: StoredImage,
    image_id: &ImageId,
    source: &SkeetStore,
    target: &SkeetStore,
) -> RedriveOutcome {
    let remote_image = match target.get_by_id(image_id).await {
        Ok(Some(img)) => img,
        Ok(None) => {
            warn!(image_id = %image_id, "exists check passed but get_by_id returned nothing");
            return RedriveOutcome::Failed;
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to fetch remote image for comparison");
            return RedriveOutcome::Failed;
        }
    };

    match local_image.content_matches(&remote_image) {
        Ok(true) => {
            info!(image_id = %image_id, "content matches remote, deleting from fallback");
            match source.delete_by_id(image_id).await {
                Ok(()) => RedriveOutcome::VerifiedAndDeleted,
                Err(e) => {
                    error!(image_id = %image_id, error = %e, "failed to delete from fallback");
                    RedriveOutcome::Failed
                }
            }
        }
        Ok(false) => {
            warn!(image_id = %image_id, "content MISMATCH with remote");
            RedriveOutcome::ContentMismatch
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to compare content");
            RedriveOutcome::Failed
        }
    }
}

async fn upload(
    image: StoredImage,
    image_id: &ImageId,
    target: &SkeetStore,
) -> RedriveOutcome {
    let record: ImageRecord = image.into();

    match target.add(&record).await {
        Ok(()) => {
            info!(image_id = %image_id, "uploaded to target");
            RedriveOutcome::Uploaded
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to upload");
            RedriveOutcome::Failed
        }
    }
}
