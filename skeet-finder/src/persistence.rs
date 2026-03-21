use skeet_store::{ImageRecord, SkeetStore};
use tracing::{info, instrument, warn};

#[instrument(skip(store, record, saved_count), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
pub async fn save(store: &SkeetStore, record: &ImageRecord, saved_count: &mut u64) {
    match store.add(record).await {
        Ok(()) => {
            *saved_count += 1;
            info!(
                saved = *saved_count,
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
