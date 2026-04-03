#![warn(clippy::all, clippy::nursery)]

use clap::{Parser, ValueEnum};
use skeet_store::{ImageId, ImageRecord, SkeetStore, StoreArgs, StoreError, StoredImage};
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

#[derive(Debug, thiserror::Error)]
enum RedriveError {
    #[error("store operation failed: {0}")]
    Store(#[from] StoreError),
}

enum VerifyResult {
    Match,
    NotFound,
    Mismatch,
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

    let result: Result<RedriveOutcome, RedriveError> = async {
        match mode {
            Mode::Upload => {
                if target.exists(&image_id).await? {
                    info!(image_id = %image_id, "already exists in target, skipping");
                    Ok(RedriveOutcome::AlreadyExists)
                } else {
                    upload(image, &image_id, target).await?;
                    Ok(RedriveOutcome::Uploaded)
                }
            }
            Mode::UploadAndDelete => match verify(&image, &image_id, target).await? {
                VerifyResult::Match => {
                    delete(&image_id, source).await?;
                    Ok(RedriveOutcome::VerifiedAndDeleted)
                }
                VerifyResult::NotFound => {
                    upload(image, &image_id, target).await?;
                    delete(&image_id, source).await?;
                    Ok(RedriveOutcome::Uploaded)
                }
                VerifyResult::Mismatch => Ok(RedriveOutcome::ContentMismatch),
            },
        }
    }
    .await;

    match result {
        Ok(outcome) => outcome,
        Err(e) => {
            error!(image_id = %image_id, error = %e, "redrive failed");
            RedriveOutcome::Failed
        }
    }
}

async fn upload(
    image: StoredImage,
    image_id: &ImageId,
    target: &SkeetStore,
) -> Result<(), RedriveError> {
    let record: ImageRecord = image.into();
    target.add(&record).await?;
    info!(image_id = %image_id, "uploaded to target");
    Ok(())
}

async fn delete(image_id: &ImageId, source: &SkeetStore) -> Result<(), RedriveError> {
    source.delete_by_id(image_id).await?;
    info!(image_id = %image_id, "deleted from source");
    Ok(())
}

async fn verify(
    local_image: &StoredImage,
    image_id: &ImageId,
    target: &SkeetStore,
) -> Result<VerifyResult, RedriveError> {
    let remote_image = match target.get_by_id(image_id).await? {
        Some(img) => img,
        None => return Ok(VerifyResult::NotFound),
    };

    match local_image.content_matches(&remote_image)? {
        true => {
            info!(image_id = %image_id, "content matches remote");
            Ok(VerifyResult::Match)
        }
        false => {
            warn!(image_id = %image_id, "content MISMATCH with remote");
            Ok(VerifyResult::Mismatch)
        }
    }
}
