use crate::status::Status;
use skeet_store::{ImageRecord, SkeetStore};
use tracing::{info, instrument, warn};

/// Returns `true` if the image already exists (caller should skip saving).
async fn already_exists(store: &SkeetStore, record: &ImageRecord) -> bool {
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

#[instrument(skip(store, record, status), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
pub async fn save(store: &SkeetStore, record: &ImageRecord, status: &mut Status) {
    if already_exists(store, record).await {
        return;
    }

    match store.add(record).await {
        Ok(()) => {
            status.record_saved();
            info!(
                saved = status.saved_count(),
                skeet_id = %record.skeet_id,
                zone = %record.zone,
                "saved image"
            );
        }
        Err(e) => {
            warn!(error = %e, "failed to save image to store");
        }
    }
}
