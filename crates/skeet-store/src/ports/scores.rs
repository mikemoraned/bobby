use std::collections::HashMap;

use async_trait::async_trait;
use shared::ImageId;

use crate::{ModelScore, StoreError};

/// Refine scores: the per-image [`ModelScore`] (a score paired with the
/// `ModelVersion` that produced it). Aggregate counts over the scores table live
/// on the [`Statistics`](crate::Statistics) port.
#[async_trait]
pub trait Scores: Send + Sync {
    async fn batch_upsert_scores(&self, scores: &[(ImageId, ModelScore)])
    -> Result<(), StoreError>;

    /// Upsert a single score — a one-row convenience over
    /// [`Scores::batch_upsert_scores`]; implementors need only provide the batch form.
    async fn upsert_score(&self, image_id: &ImageId, scored: ModelScore) -> Result<(), StoreError> {
        self.batch_upsert_scores(&[(image_id.clone(), scored)])
            .await
    }

    async fn get_score(&self, image_id: &ImageId) -> Result<Option<ModelScore>, StoreError>;
    async fn list_scores_for_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, ModelScore>, StoreError>;
}
