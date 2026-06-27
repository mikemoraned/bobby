use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use shared::{DiscoveredAt, ImageId, SkeetId};

use crate::{ImageRecord, StoreError, StoredImage, StoredImageSummary, StoredOriginal};

/// Pruned images: the stored image records, their summaries, and the cursor-based
/// listing over them. Keyed throughout by [`ImageId`].
#[async_trait]
pub trait Images: Send + Sync {
    async fn add(&self, record: &ImageRecord) -> Result<(), StoreError>;
    async fn get_by_id(&self, image_id: &ImageId) -> Result<Option<StoredImage>, StoreError>;
    async fn get_by_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, StoredImage>, StoreError>;
    async fn get_originals_by_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, StoredOriginal>, StoreError>;
    async fn exists(&self, image_id: &ImageId) -> Result<bool, StoreError>;
    async fn delete_by_id(&self, image_id: &ImageId) -> Result<(), StoreError>;
    async fn count(&self) -> Result<usize, StoreError>;
    /// The earliest `discovered_at` over all stored images, or `None` when empty.
    async fn oldest_discovered_at(&self) -> Result<Option<DiscoveredAt>, StoreError>;
    /// The latest `discovered_at` over all stored images, or `None` when empty.
    async fn newest_discovered_at(&self) -> Result<Option<DiscoveredAt>, StoreError>;
    /// Count images whose `discovered_at` falls in the half-open window
    /// `[start, end)`.
    async fn count_in_interval(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<u64, StoreError>;
    async fn list_all_summaries(&self) -> Result<Vec<StoredImageSummary>, StoreError>;
    /// Up to `limit` summaries ordered by `discovered_at` desc, starting strictly
    /// before `before` (or the newest row if `before` is `None`). The returned
    /// cursor is the `before` to pass for the next page, or `None` at end of data.
    async fn list_summaries_page(
        &self,
        before: Option<DiscoveredAt>,
        limit: usize,
    ) -> Result<(Vec<StoredImageSummary>, Option<DiscoveredAt>), StoreError>;
    async fn unique_skeet_ids(&self) -> Result<Vec<SkeetId>, StoreError>;
    async fn list_all_image_ids_by_most_recent(
        &self,
        since: Option<&DiscoveredAt>,
    ) -> Result<Vec<ImageId>, StoreError>;
}
