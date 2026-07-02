use skeet_store::{ImageRecord, Images};
use tracing::{info, instrument, warn};

/// Whether a `save` call persisted a fresh record, or skipped it (the image
/// already existed, or the write failed and was logged).
pub enum SaveOutcome {
    Saved,
    Skipped,
}

/// Returns `true` if the image already exists (caller should skip saving).
async fn already_exists(store: &impl Images, record: &ImageRecord) -> bool {
    match store.exists(&record.image_id).await {
        Ok(true) => {
            info!(image_id = %record.image_id, "image already exists, skipping");
            true
        }
        Ok(false) => false,
        Err(e) => {
            warn!(error = %e, "failed to check image existence, attempting save anyway");
            false
        }
    }
}

#[instrument(skip(store, record), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
pub async fn save(store: &impl Images, record: &ImageRecord) -> SaveOutcome {
    if already_exists(store, record).await {
        return SaveOutcome::Skipped;
    }

    match store.add(record).await {
        Ok(()) => SaveOutcome::Saved,
        Err(e) => {
            warn!(error = %e, "failed to save image to store");
            SaveOutcome::Skipped
        }
    }
}
