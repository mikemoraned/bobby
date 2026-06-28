use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::ImageId;
use tracing::instrument;

use super::decode::{decode_rows, decode_score_row, score_columns};
use super::query::{col_eq, col_in, execute_query};
use super::schema::{TableName, images_score_v2_schema};
use crate::{ModelScore, Scores, SkeetStore, StoreError};

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

}
