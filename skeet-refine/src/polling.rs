use std::sync::Arc;

use image::DynamicImage;
use skeet_store::{ImageId, ModelVersion, SkeetStore, StoreError, StoredOriginal, TABLE_NAME};
use tracing::info;

/// A staging buffer of images to be scored together in one parallel dispatch.
#[derive(Default)]
pub struct Batch {
    pub ids: Vec<ImageId>,
    pub images: Vec<DynamicImage>,
}

impl Batch {
    pub const fn len(&self) -> usize {
        self.ids.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

impl From<Vec<StoredOriginal>> for Batch {
    fn from(originals: Vec<StoredOriginal>) -> Self {
        let mut ids = Vec::with_capacity(originals.len());
        let mut images = Vec::with_capacity(originals.len());
        for s in originals {
            ids.push(s.summary.image_id);
            images.push(s.image);
        }
        Self { ids, images }
    }
}

pub struct PollingBatchSource {
    store: Arc<SkeetStore>,
    model_version: ModelVersion,
    last_images_version: Option<u64>,
}

impl PollingBatchSource {
    pub const fn new(store: Arc<SkeetStore>, model_version: ModelVersion) -> Self {
        Self {
            store,
            model_version,
            last_images_version: None,
        }
    }

    /// Fetch unscored images for this tick.
    ///
    /// Returns an empty `Batch` if the `images` table version hasn't changed since
    /// the last call — skipping the expensive full-table scan. On cold start
    /// (first call) always runs the scan regardless.
    pub async fn fetch(&mut self) -> Result<Batch, StoreError> {
        let versions = self.store.table_versions().await?;
        let images_version = versions
            .iter()
            .find(|(name, _)| *name == TABLE_NAME)
            .map(|(_, v)| *v);

        if self.last_images_version.is_some() && images_version == self.last_images_version {
            return Ok(Batch::default());
        }

        let unscored_ids = self
            .store
            .list_unscored_image_ids_for_version(&self.model_version)
            .await?;

        self.last_images_version = images_version;

        if unscored_ids.is_empty() {
            return Ok(Batch::default());
        }

        info!(count = unscored_ids.len(), "found unscored images");

        let originals = self.store.get_originals_by_ids(&unscored_ids).await?;
        Ok(Batch::from(originals))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skeet_store::test_utils::{make_record, open_temp_store};

    #[tokio::test]
    async fn cold_start_always_fetches() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        store.add(&make_record("cold1", 10, 0, 0)).await.expect("add");

        let mut source = PollingBatchSource::new(store, ModelVersion::from("v1"));
        let batch = source.fetch().await.expect("fetch");
        assert_eq!(batch.len(), 1);
    }

    #[tokio::test]
    async fn unchanged_version_returns_empty_batch() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        store.add(&make_record("same1", 10, 0, 0)).await.expect("add");

        let mut source = PollingBatchSource::new(store, ModelVersion::from("v1"));
        let _ = source.fetch().await.expect("first fetch");
        let batch = source.fetch().await.expect("second fetch");
        assert!(batch.is_empty(), "expected early-abort on unchanged version");
    }

    #[tokio::test]
    async fn changed_version_fetches_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(open_temp_store(&dir).await);
        store.add(&make_record("chg1", 10, 0, 0)).await.expect("add");

        let mut source = PollingBatchSource::new(store.clone(), ModelVersion::from("v1"));
        let first = source.fetch().await.expect("first fetch");
        assert_eq!(first.len(), 1);

        store.add(&make_record("chg2", 20, 0, 0)).await.expect("add");

        let second = source.fetch().await.expect("second fetch");
        assert_eq!(second.len(), 2, "both images should be unscored");
    }
}
