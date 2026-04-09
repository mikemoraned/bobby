use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use lancedb::query::QueryBase;
use tracing::{debug, info, instrument};

use crate::arrow_utils::typed_column;
use crate::lancedb_utils::execute_query;
use crate::schema::images_score_v2_schema;
use crate::stored::batches_to_summaries;
use crate::types::ImageId;
use crate::{ModelVersion, Score, SkeetStore, StoredImageSummary, StoreError};

pub struct ScoresCache {
    pub version: u64,
    pub scores: HashMap<ImageId, Score>,
}

impl SkeetStore {
    const MAX_SCORED_SUMMARIES: usize = 100;

    #[instrument(skip(self))]
    pub async fn upsert_score(
        &self,
        image_id: &ImageId,
        score: &Score,
        model_version: &ModelVersion,
    ) -> Result<(), StoreError> {
        self.scores_table
            .delete(&format!("image_id = '{image_id}'"))
            .await?;

        let schema = images_score_v2_schema();
        let image_id_str = image_id.to_string();
        let model_version_str = model_version.to_string();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![image_id_str.as_str()])),
                Arc::new(Float32Array::from(vec![f32::from(*score)])),
                Arc::new(StringArray::from(vec![model_version_str.as_str()])),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.scores_table.add(batches).execute().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn batch_upsert_scores(
        &self,
        scores: &[(ImageId, Score, ModelVersion)],
    ) -> Result<(), StoreError> {
        if scores.is_empty() {
            return Ok(());
        }

        for (image_id, _, _) in scores {
            self.scores_table
                .delete(&format!("image_id = '{image_id}'"))
                .await?;
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

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.scores_table.add(batches).execute().await?;
        self.compact_if_needed().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_score(
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
        let score = Score::new(scores.value(0))
            .map_err(|e| StoreError::ValidationFailed(e.to_string()))?;
        let model_version: ModelVersion = model_versions
            .value(0)
            .parse()
            .expect("ModelVersion parse is infallible");
        Ok(Some((score, model_version)))
    }

    #[instrument(skip(self))]
    pub async fn list_unscored_image_ids_for_version(
        &self,
        model_version: &ModelVersion,
    ) -> Result<Vec<ImageId>, StoreError> {
        let all_ids = self.list_all_image_ids_by_most_recent().await?;

        let version_str = model_version.to_string();
        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if(format!("model_version = '{version_str}'"));
        let scored_batches = execute_query(&scored_query, "list_unscored:scored_ids").await?;

        let mut scored_with_current_version = std::collections::HashSet::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                scored_with_current_version.insert(image_ids.value(i).to_string());
            }
        }

        Ok(all_ids
            .into_iter()
            .filter(|id| !scored_with_current_version.contains(&id.to_string()))
            .collect())
    }

    #[instrument(skip(self))]
    pub async fn list_scored_summaries_by_score(
        &self,
        limit: usize,
        max_age_hours: Option<u64>,
    ) -> Result<Vec<(StoredImageSummary, Score)>, StoreError> {
        if limit > Self::MAX_SCORED_SUMMARIES {
            return Err(StoreError::LimitExceeded {
                requested: limit,
                maximum: Self::MAX_SCORED_SUMMARIES,
            });
        }

        let recent_ids = self.find_recent_image_ids(max_age_hours).await?;
        let top_scores = self.read_top_scores(limit, &recent_ids).await?;

        if top_scores.is_empty() {
            return Ok(Vec::new());
        }

        self.fetch_summaries_for_scores(&top_scores).await
    }

    #[instrument(skip(self))]
    async fn find_recent_image_ids(
        &self,
        max_age_hours: Option<u64>,
    ) -> Result<Option<std::collections::HashSet<ImageId>>, StoreError> {
        let Some(hours) = max_age_hours else {
            return Ok(None);
        };
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours as i64);
        let cutoff_us = cutoff.timestamp_micros();
        let filter = format!(
            "discovered_at >= arrow_cast({cutoff_us}, 'Timestamp(Microsecond, Some(\"UTC\"))')"
        );
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if(filter);
        let batches = execute_query(&query, "find_recent_image_ids").await?;
        let mut ids = std::collections::HashSet::new();
        for batch in &batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                let image_id: ImageId = image_ids.value(i).parse()?;
                ids.insert(image_id);
            }
        }
        info!(recent_image_ids = ids.len(), "filtered images by age");
        Ok(Some(ids))
    }

    #[instrument(skip(self, recent_ids))]
    async fn read_top_scores(
        &self,
        limit: usize,
        recent_ids: &Option<std::collections::HashSet<ImageId>>,
    ) -> Result<Vec<(Score, ImageId)>, StoreError> {
        let all_scores_map = self.cached_scores().await?;

        let mut all_scores: Vec<(Score, ImageId)> = all_scores_map
            .iter()
            .filter(|(id, _)| {
                recent_ids
                    .as_ref()
                    .is_none_or(|recent| recent.contains(id))
            })
            .map(|(id, score)| (*score, id.clone()))
            .collect();
        info!(score_rows = all_scores.len(), "read scores (after age filter)");

        all_scores.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_scores.truncate(limit);
        Ok(all_scores)
    }

    #[instrument(skip(self))]
    async fn cached_scores(&self) -> Result<HashMap<ImageId, Score>, StoreError> {
        let current_version = self.scores_table.version().await?;

        // Fast path: check if cache is still valid
        {
            let cache = self.scores_cache.read().await;
            if let Some(ref cached) = *cache
                && cached.version == current_version
            {
                debug!(version = current_version, "scores cache hit");
                return Ok(cached.scores.clone());
            }
        }

        // Slow path: full scan and cache update
        debug!(version = current_version, "scores cache miss — full scan");
        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id", "score"]));
        let scored_batches = execute_query(&scored_query, "cached_scores:full_scan").await?;

        let mut scores = HashMap::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let score_vals = typed_column::<Float32Array>(batch, "score")?;
            for i in 0..batch.num_rows() {
                let image_id: ImageId = image_ids.value(i).parse()?;
                let score = Score::new(score_vals.value(i))
                    .map_err(|e| StoreError::ValidationFailed(e.to_string()))?;
                scores.insert(image_id, score);
            }
        }
        info!(score_rows = scores.len(), version = current_version, "scores cache refreshed");

        let result = scores.clone();
        *self.scores_cache.write().await = Some(ScoresCache {
            version: current_version,
            scores,
        });
        Ok(result)
    }

    #[instrument(skip(self, top_scores))]
    async fn fetch_summaries_for_scores(
        &self,
        top_scores: &[(Score, ImageId)],
    ) -> Result<Vec<(StoredImageSummary, Score)>, StoreError> {
        let score_map: HashMap<&ImageId, Score> = top_scores
            .iter()
            .map(|(s, id)| (id, *s))
            .collect();

        let in_list = top_scores
            .iter()
            .map(|(_, id)| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("image_id IN ({in_list})");

        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "skeet_id",
                "discovered_at",
                "original_at",
                "archetype",
                "config_version",
                "detected_text",
            ]))
            .only_if(filter);
        let batches = execute_query(&query, "fetch_summaries_for_scores").await?;
        let summaries = batches_to_summaries(&batches)?;
        info!(summary_rows = summaries.len(), "read summaries for top scores");

        let mut scored: Vec<(StoredImageSummary, Score)> = summaries
            .into_iter()
            .filter_map(|s| {
                let score = score_map.get(&s.image_id).copied();
                score.map(|sc| (s, sc))
            })
            .collect();

        info!(matched_rows = scored.len(), "joined scores with summaries");

        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(scored)
    }

    #[instrument(skip(self))]
    pub async fn list_scores_for_ids(
        &self,
        image_ids: &[&str],
    ) -> Result<HashMap<ImageId, Score>, StoreError> {
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
            .select(lancedb::query::Select::columns(&["image_id", "score"]))
            .only_if(filter);
        let batches = execute_query(&query, "list_scores_for_ids").await?;

        let mut score_map = HashMap::new();
        for batch in &batches {
            let ids = typed_column::<StringArray>(batch, "image_id")?;
            let scores = typed_column::<Float32Array>(batch, "score")?;
            for i in 0..batch.num_rows() {
                let score = Score::new(scores.value(i))
                    .map_err(|e| StoreError::ValidationFailed(e.to_string()))?;
                let image_id: ImageId = ids.value(i).parse()?;
                score_map.insert(image_id, score);
            }
        }
        Ok(score_map)
    }
}
