use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_array::{Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::ImageId;
use tracing::instrument;

use crate::arrow_utils::typed_column;
use crate::lancedb_utils::execute_query;
use crate::schema::images_score_v2_schema;
use crate::{ModelVersion, Score, SkeetStore, StoreError};

/// Refine scores: the per-image model score paired with the `ModelVersion` that
/// produced it, plus aggregate counts over the scores table.
#[async_trait]
pub trait Scores: Send + Sync {
    async fn batch_upsert_scores(
        &self,
        scores: &[(ImageId, Score, ModelVersion)],
    ) -> Result<(), StoreError>;

    /// Upsert a single score — a one-row convenience over
    /// [`Scores::batch_upsert_scores`]; implementors need only provide the batch form.
    async fn upsert_score(
        &self,
        image_id: &ImageId,
        score: &Score,
        model_version: &ModelVersion,
    ) -> Result<(), StoreError> {
        self.batch_upsert_scores(&[(image_id.clone(), *score, model_version.clone())])
            .await
    }

    async fn get_score(
        &self,
        image_id: &ImageId,
    ) -> Result<Option<(Score, ModelVersion)>, StoreError>;
    async fn list_scores_for_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, (Score, ModelVersion)>, StoreError>;
    async fn count_scored_images(
        &self,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<usize, StoreError>;
    async fn count_scores_by_model_version(&self) -> Result<HashMap<String, usize>, StoreError>;
}

#[async_trait]
impl Scores for SkeetStore {
    #[instrument(skip(self))]
    async fn batch_upsert_scores(
        &self,
        scores: &[(ImageId, Score, ModelVersion)],
    ) -> Result<(), StoreError> {
        if scores.is_empty() {
            return Ok(());
        }

        let schema = images_score_v2_schema();
        let image_ids: Vec<String> = scores.iter().map(|(id, _, _)| id.to_string()).collect();
        let score_vals: Vec<f32> = scores.iter().map(|(_, s, _)| f32::from(*s)).collect();
        let model_versions: Vec<String> = scores.iter().map(|(_, _, mv)| mv.to_string()).collect();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(
                    image_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(Float32Array::from(score_vals)),
                Arc::new(StringArray::from(
                    model_versions
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>(),
                )),
            ],
        )?;

        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let mut builder = self.scores_table.merge_insert(&["image_id"]);
        builder.when_matched_update_all(None);
        builder.when_not_matched_insert_all();
        builder.execute(Box::new(reader)).await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_score(
        &self,
        image_id: &ImageId,
    ) -> Result<Option<(Score, ModelVersion)>, StoreError> {
        let query = self
            .scores_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1);
        let batches = execute_query(&query, "get_score").await?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Ok(None);
        }

        let scores = typed_column::<Float32Array>(&batches[0], "score")?;
        let model_versions = typed_column::<StringArray>(&batches[0], "model_version")?;
        let score = Score::new(scores.value(0))?;
        let model_version = ModelVersion::from(model_versions.value(0));
        Ok(Some((score, model_version)))
    }

    #[instrument(skip(self))]
    async fn list_scores_for_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, (Score, ModelVersion)>, StoreError> {
        if image_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let in_list = image_ids
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("image_id IN ({in_list})");

        let query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "score",
                "model_version",
            ]))
            .only_if(filter);
        let batches = execute_query(&query, "list_scores_for_ids").await?;

        let mut score_map = HashMap::new();
        for batch in &batches {
            let ids = typed_column::<StringArray>(batch, "image_id")?;
            let scores = typed_column::<Float32Array>(batch, "score")?;
            let model_versions = typed_column::<StringArray>(batch, "model_version")?;
            for i in 0..batch.num_rows() {
                let score = Score::new(scores.value(i))?;
                let image_id: ImageId = ids.value(i).parse()?;
                let model_version = ModelVersion::from(model_versions.value(i));
                score_map.insert(image_id, (score, model_version));
            }
        }
        Ok(score_map)
    }

    /// Count the distinct images that have a score from a *known* model version —
    /// the "images examined" total, i.e. images that made it through refine
    /// scoring. Each image is counted once; scores from unregistered model
    /// versions are excluded, mirroring the feed read path (see
    /// `docs/versioning.md`).
    ///
    /// Scans the scores table fresh rather than reading the scores cache: callers
    /// want the current total, and the cache may lag the live table.
    #[instrument(skip(self, known_versions))]
    async fn count_scored_images(
        &self,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<usize, StoreError> {
        let query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "model_version",
            ]));
        let batches = execute_query(&query, "count_scored_images").await?;

        // Dedupe by image id (last row wins) so an image scored more than once is
        // counted once, with its latest model version deciding known-ness.
        let mut latest_version: HashMap<ImageId, ModelVersion> = HashMap::new();
        for batch in &batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let model_versions = typed_column::<StringArray>(batch, "model_version")?;
            for i in 0..batch.num_rows() {
                let image_id: ImageId = image_ids.value(i).parse()?;
                latest_version.insert(image_id, ModelVersion::from(model_versions.value(i)));
            }
        }

        Ok(latest_version
            .values()
            .filter(|mv| known_versions.contains(mv))
            .count())
    }

    /// Scan all scores and return a count per distinct `model_version`.
    #[instrument(skip(self))]
    async fn count_scores_by_model_version(&self) -> Result<HashMap<String, usize>, StoreError> {
        let query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["model_version"]));
        let batches = execute_query(&query, "count_scores_by_model_version").await?;

        let mut counts: HashMap<String, usize> = HashMap::new();
        for batch in &batches {
            let model_versions = typed_column::<StringArray>(batch, "model_version")?;
            for i in 0..batch.num_rows() {
                *counts
                    .entry(model_versions.value(i).to_string())
                    .or_insert(0) += 1;
            }
        }
        Ok(counts)
    }
}
