use std::collections::HashSet;

use async_trait::async_trait;
use shared::{DiscoveredAt, ImageId};

use crate::{ModelVersion, Score, StoreError, StoredImageSummary};

/// Cross-table read-models that join the images and scores tables.
///
/// These — the scored feed views and the unscored backlog — belong to neither
/// the [`crate::Images`] nor [`crate::Scores`] port because each one reads both
/// tables.
#[async_trait]
pub trait ScoredView: Send + Sync {
    /// Image IDs that have no score — regardless of which `model_version`
    /// produced any existing score.
    async fn list_unscored_image_ids(
        &self,
        since: Option<&DiscoveredAt>,
    ) -> Result<Vec<ImageId>, StoreError>;
    /// Top scored summaries, considering only scores whose `model_version` is in
    /// `known_versions`. Unknown versions are discarded at read time — see
    /// `docs/versioning.md`.
    async fn list_scored_summaries_by_score(
        &self,
        limit: usize,
        max_age_hours: Option<u64>,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<Vec<(StoredImageSummary, Score, ModelVersion)>, StoreError>;
    /// All scored summaries published (`original_at`) at or after `cutoff` whose
    /// `model_version` is in `known_versions`, **uncapped** and unordered (the
    /// caller orders). Unlike [`ScoredView::list_scored_summaries_by_score`] there
    /// is no top-N truncation, so a recent low-score image is never dropped.
    async fn list_scored_summaries_published_since(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<Vec<(StoredImageSummary, Score, ModelVersion)>, StoreError>;
}
