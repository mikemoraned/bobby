#![warn(clippy::all, clippy::nursery)]

use clap::{Parser, ValueEnum};
use skeet_store::{ImageId, ImageRecord, SkeetStore, StoreArgs, StoredImage};
use tracing::{error, info, warn};

#[derive(Debug, Clone, ValueEnum)]
enum Mode {
    /// Upload new images, verify and delete existing ones from source
    UploadAndDelete,
    /// Upload new images, skip existing ones
    Upload,
}

#[derive(Parser)]
#[command(about = "Redrive images from a local fallback store to a remote store")]
struct Args {
    /// Local fallback store path (source)
    #[arg(long)]
    source_store_path: String,

    /// Remote store (target)
    #[command(flatten)]
    target: StoreArgs,

    /// Redrive mode
    #[arg(long)]
    mode: Mode,

    /// Upload most recently discovered images first
    #[arg(long, default_value_t = false)]
    most_recent_first: bool,
}

enum RedriveOutcome {
    Uploaded,
    VerifiedAndDeleted,
    AlreadyExists,
    ContentMismatch,
    Failed,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let source = SkeetStore::open(&args.source_store_path, vec![], None).await?;
    info!(path = %args.source_store_path, "opened source store");

    let target = args.target.open_store().await?;
    target.validate().await?;
    info!("target store validation passed");

    let images = if args.most_recent_first {
        source.list_all_by_most_recent().await?
    } else {
        source.list_all().await?
    };
    info!(count = images.len(), most_recent_first = args.most_recent_first, mode = ?args.mode, "found images in source store");

    let mut uploaded = 0u64;
    let mut verified_and_deleted = 0u64;
    let mut already_exists = 0u64;
    let mut content_mismatch = 0u64;
    let mut failed = 0u64;

    for image in images {
        let outcome = redrive_image(image, &source, &target, &args.mode).await;
        match outcome {
            RedriveOutcome::Uploaded => uploaded += 1,
            RedriveOutcome::VerifiedAndDeleted => verified_and_deleted += 1,
            RedriveOutcome::AlreadyExists => already_exists += 1,
            RedriveOutcome::ContentMismatch => content_mismatch += 1,
            RedriveOutcome::Failed => failed += 1,
        }
    }

    info!(
        uploaded,
        verified_and_deleted,
        already_exists,
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
    mode: &Mode,
) -> RedriveOutcome {
    let image_id = image.summary.image_id.clone();

    let exists = match target.exists(&image_id).await {
        Ok(exists) => exists,
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to check existence");
            return RedriveOutcome::Failed;
        }
    };

    match (exists, mode) {
        (true, Mode::Upload) => {
            info!(image_id = %image_id, "already exists in target, skipping");
            RedriveOutcome::AlreadyExists
        }
        (true, Mode::UploadAndDelete) => match verify(&image, &image_id, target).await {
            VerifyResult::Match => {
                if delete(&image_id, source).await {
                    RedriveOutcome::VerifiedAndDeleted
                } else {
                    RedriveOutcome::Failed
                }
            }
            VerifyResult::Mismatch => RedriveOutcome::ContentMismatch,
            VerifyResult::Failed => RedriveOutcome::Failed,
        },
        (false, Mode::Upload) => {
            if upload(image, &image_id, target).await {
                RedriveOutcome::Uploaded
            } else {
                RedriveOutcome::Failed
            }
        }
        (false, Mode::UploadAndDelete) => {
            if upload(image, &image_id, target).await {
                if delete(&image_id, source).await {
                    RedriveOutcome::Uploaded
                } else {
                    RedriveOutcome::Failed
                }
            } else {
                RedriveOutcome::Failed
            }
        }
    }
}

async fn upload(image: StoredImage, image_id: &ImageId, target: &SkeetStore) -> bool {
    let record: ImageRecord = image.into();
    match target.add(&record).await {
        Ok(()) => {
            info!(image_id = %image_id, "uploaded to target");
            true
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to upload");
            false
        }
    }
}

async fn delete(image_id: &ImageId, source: &SkeetStore) -> bool {
    match source.delete_by_id(image_id).await {
        Ok(()) => {
            info!(image_id = %image_id, "deleted from source");
            true
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to delete from source");
            false
        }
    }
}

enum VerifyResult {
    Match,
    Mismatch,
    Failed,
}

async fn verify(
    local_image: &StoredImage,
    image_id: &ImageId,
    target: &SkeetStore,
) -> VerifyResult {
    let remote_image = match target.get_by_id(image_id).await {
        Ok(Some(img)) => img,
        Ok(None) => {
            warn!(image_id = %image_id, "exists check passed but get_by_id returned nothing");
            return VerifyResult::Failed;
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to fetch remote image for comparison");
            return VerifyResult::Failed;
        }
    };

    match local_image.content_matches(&remote_image) {
        Ok(true) => {
            info!(image_id = %image_id, "content matches remote");
            VerifyResult::Match
        }
        Ok(false) => {
            warn!(image_id = %image_id, "content MISMATCH with remote");
            VerifyResult::Mismatch
        }
        Err(e) => {
            error!(image_id = %image_id, error = %e, "failed to compare content");
            VerifyResult::Failed
        }
    }
}
