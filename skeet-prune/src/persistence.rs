use crate::status::Status;
use skeet_store::{ImageRecord, SkeetStore};
use tracing::{info, instrument, warn};

#[instrument(skip(store, record, status), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
pub async fn save(store: &SkeetStore, record: &ImageRecord, status: &mut Status) {
    match store.exists(&record.image_id).await {
        Ok(true) => {
            info!(image_id = %record.image_id, "image already exists, skipping");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            warn!(error = %e, "failed to check image existence, attempting save anyway");
        }
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

#[instrument(skip(primary, fallback, record, status), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
pub async fn save_with_fallback(
    primary: &SkeetStore,
    fallback: &SkeetStore,
    record: &ImageRecord,
    status: &mut Status,
) {
    match primary.exists(&record.image_id).await {
        Ok(true) => {
            info!(image_id = %record.image_id, "image already exists, skipping");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            warn!(error = %e, "failed to check image existence, attempting save anyway");
        }
    }

    match primary.add(record).await {
        Ok(()) => {
            status.record_saved_remote();
            info!(
                saved = status.saved_count(),
                skeet_id = %record.skeet_id,
                zone = %record.zone,
                "saved image to remote"
            );
        }
        Err(e) => {
            warn!(error = %e, "failed to save to remote, trying fallback");
            match fallback.add(record).await {
                Ok(()) => {
                    status.record_saved_fallback();
                    info!(
                        saved = status.saved_count(),
                        skeet_id = %record.skeet_id,
                        zone = %record.zone,
                        "saved image to fallback"
                    );
                }
                Err(fallback_err) => {
                    warn!(error = %fallback_err, "failed to save to fallback store");
                }
            }
        }
    }
}
