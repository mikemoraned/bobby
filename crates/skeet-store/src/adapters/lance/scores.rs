use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_array::{Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::ImageId;
use tracing::instrument;

use super::arrow::typed_column;
use super::decode::{decode_rows, decode_score_row, score_columns};
use super::query::{col_eq, col_in, execute_query};
use super::schema::{TableName, images_score_v2_schema};
use crate::{ModelScore, ModelVersion, Scores, SkeetStore, StoreError};

#[async_trait]
impl Scores for SkeetStore {
    #[instrument(skip(self))]
    async fn batch_upsert_scores(
        &self,
        scores: &[(ImageId, ModelScore)],
    ) -> Result<(), StoreError> {
        if scores.is_empty() {
            return Ok(());
        }

        let schema = images_score_v2_schema();
        let image_ids: Vec<String> = scores.iter().map(|(id, _)| id.to_string()).collect();
        let score_vals: Vec<f32> = scores.iter().map(|(_, ms)| f32::from(ms.score)).collect();
        let model_versions: Vec<String> = scores
            .iter()
            .map(|(_, ms)| ms.model_version.to_string())
            .collect();

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
        let mut builder = self.table(TableName::Scores).merge_insert(&["image_id"]);
        builder.when_matched_update_all(None);
        builder.when_not_matched_insert_all();
        builder.execute(Box::new(reader)).await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_score(&self, image_id: &ImageId) -> Result<Option<ModelScore>, StoreError> {
        let query = self
            .table(TableName::Scores)
            .query()
            .only_if_expr(col_eq("image_id", image_id.to_string()))
            .limit(1);
        let batches = execute_query(&query, "get_score").await?;

        Ok(decode_rows(&batches, score_columns, decode_score_row)?
            .into_iter()
            .next()
            .map(|(_, ms)| ms))
    }

    #[instrument(skip(self))]
    async fn list_scores_for_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, ModelScore>, StoreError> {
        if image_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let query = self
            .table(TableName::Scores)
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "score",
                "model_version",
            ]))
            .only_if_expr(col_in(
                "image_id",
                image_ids.iter().map(|id| id.to_string()),
            ));
        let batches = execute_query(&query, "list_scores_for_ids").await?;

        Ok(decode_rows(&batches, score_columns, decode_score_row)?
            .into_iter()
            .collect())
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
            .table(TableName::Scores)
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "model_version",
            ]));
        let batches = execute_query(&query, "count_scored_images").await?;

        // Dedupe by image id (last row wins) so an image scored more than once is
        // counted once, with its latest model version deciding known-ness.
        let latest_version: HashMap<ImageId, ModelVersion> = decode_rows(
            &batches,
            |batch| {
                Ok((
                    typed_column::<StringArray>(batch, "image_id")?,
                    typed_column::<StringArray>(batch, "model_version")?,
                ))
            },
            |(image_ids, model_versions), i| {
                Ok((
                    image_ids.value(i).parse::<ImageId>()?,
                    ModelVersion::from(model_versions.value(i)),
                ))
            },
        )?
        .into_iter()
        .collect();

        Ok(latest_version
            .values()
            .filter(|mv| known_versions.contains(mv))
            .count())
    }

    /// Scan all scores and return a count per distinct `model_version`.
    #[instrument(skip(self))]
    async fn count_scores_by_model_version(&self) -> Result<HashMap<String, usize>, StoreError> {
        let query = self
            .table(TableName::Scores)
            .query()
            .select(lancedb::query::Select::columns(&["model_version"]));
        let batches = execute_query(&query, "count_scores_by_model_version").await?;

        let mut counts: HashMap<String, usize> = HashMap::new();
        for model_version in decode_rows(
            &batches,
            |batch| typed_column::<StringArray>(batch, "model_version"),
            |col, i| Ok(col.value(i).to_string()),
        )? {
            *counts.entry(model_version).or_insert(0) += 1;
        }
        Ok(counts)
    }
}
