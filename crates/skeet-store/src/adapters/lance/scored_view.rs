use std::collections::{HashMap, HashSet};

use arrow_array::StringArray;
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::{DiscoveredAt, ImageId};
use tracing::{debug, info, instrument};

use super::arrow::typed_column;
use super::decode::{batches_to_summaries, decode_rows, decode_score_row, score_columns};
use super::query::{col_at_or_after_micros, col_in, execute_query};
use super::schema::TableName;
use crate::model::ScoresMap;
use crate::{
    Images, ModelScore, ModelVersion, ScoredSummary, ScoredView, SkeetStore, StoreError,
    TableVersions,
};

#[async_trait]
impl ScoredView for SkeetStore {
    #[instrument(skip(self))]
    async fn list_unscored_image_ids(
        &self,
        since: Option<&DiscoveredAt>,
    ) -> Result<Vec<ImageId>, StoreError> {
        let all_ids = self.list_all_image_ids_by_most_recent(since).await?;

        let scored_query = self
            .table(TableName::Scores)
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]));
        let scored_batches = execute_query(&scored_query, "list_unscored:scored_ids").await?;

        let scored: std::collections::HashSet<String> = decode_rows(
            &scored_batches,
            |batch| typed_column::<StringArray>(batch, "image_id"),
            |col, i| Ok(col.value(i).to_string()),
        )?
        .into_iter()
        .collect();

        Ok(all_ids
            .into_iter()
            .filter(|id| !scored.contains(&id.to_string()))
            .collect())
    }

    #[instrument(skip(self, known_versions))]
    async fn list_scored_summaries_by_score(
        &self,
        limit: usize,
        max_age_hours: Option<u64>,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<Vec<ScoredSummary>, StoreError> {
        if limit > Self::MAX_SCORED_SUMMARIES {
            return Err(StoreError::LimitExceeded {
                requested: limit,
                maximum: Self::MAX_SCORED_SUMMARIES,
            });
        }

        let recent_ids = self.find_recent_image_ids(max_age_hours).await?;
        let top_scores = self
            .read_top_scores(limit, &recent_ids, known_versions)
            .await?;

        if top_scores.is_empty() {
            return Ok(Vec::new());
        }

        self.fetch_summaries_for_scores(&top_scores).await
    }

    #[instrument(skip(self, known_versions))]
    async fn list_scored_summaries_published_since(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<Vec<ScoredSummary>, StoreError> {
        let windowed_ids = self.find_image_ids_published_since(cutoff).await?;
        if windowed_ids.is_empty() {
            return Ok(Vec::new());
        }

        let all_scores = self.cached_scores().await?;
        let scored: Vec<(ModelScore, ImageId)> = windowed_ids
            .iter()
            .filter_map(|id| {
                all_scores
                    .get(id)
                    .filter(|vs| known_versions.contains(&vs.model_version))
                    .map(|vs| (vs.clone(), id.clone()))
            })
            .collect();
        if scored.is_empty() {
            return Ok(Vec::new());
        }

        self.fetch_summaries_for_scores(&scored).await
    }
}

impl SkeetStore {
    const MAX_SCORED_SUMMARIES: usize = 100;

    /// Image ids whose `original_at` (publish time) is at or after `cutoff`.
    #[instrument(skip(self))]
    async fn find_image_ids_published_since(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<HashSet<ImageId>, StoreError> {
        let query = self
            .table(TableName::Images)
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if_expr(col_at_or_after_micros(
                "original_at",
                cutoff.timestamp_micros(),
            ));
        let batches = execute_query(&query, "find_image_ids_published_since").await?;
        let ids: HashSet<ImageId> = decode_rows(
            &batches,
            |batch| typed_column::<StringArray>(batch, "image_id"),
            |col, i| Ok(col.value(i).parse()?),
        )?
        .into_iter()
        .collect();
        info!(
            windowed_image_ids = ids.len(),
            "filtered images by publish time"
        );
        Ok(ids)
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
        let query = self
            .table(TableName::Images)
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if_expr(col_at_or_after_micros(
                "discovered_at",
                cutoff.timestamp_micros(),
            ));
        let batches = execute_query(&query, "find_recent_image_ids").await?;
        let ids: std::collections::HashSet<ImageId> = decode_rows(
            &batches,
            |batch| typed_column::<StringArray>(batch, "image_id"),
            |col, i| Ok(col.value(i).parse()?),
        )?
        .into_iter()
        .collect();
        info!(recent_image_ids = ids.len(), "filtered images by age");
        Ok(Some(ids))
    }

    #[instrument(skip(self, recent_ids, known_versions))]
    async fn read_top_scores(
        &self,
        limit: usize,
        recent_ids: &Option<std::collections::HashSet<ImageId>>,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<Vec<(ModelScore, ImageId)>, StoreError> {
        let all_scores_map = self.cached_scores().await?;

        let mut all_scores: Vec<(ModelScore, ImageId)> = all_scores_map
            .iter()
            .filter(|(id, _)| recent_ids.as_ref().is_none_or(|recent| recent.contains(id)))
            .filter(|(_, vs)| known_versions.contains(&vs.model_version))
            .map(|(id, vs)| (vs.clone(), id.clone()))
            .collect();
        info!(
            score_rows = all_scores.len(),
            "read scores (after age filter)"
        );

        all_scores.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_scores.truncate(limit);
        Ok(all_scores)
    }

    #[instrument(skip(self))]
    async fn cached_scores(&self) -> Result<ScoresMap, StoreError> {
        let current_version = self.table_version(TableName::Scores.as_str()).await?;

        // Fast path: reuse the cache if it was built at this table version.
        if let Some(scores) = self
            .scores_cache
            .read()
            .await
            .get_cached_if_current(&current_version)
        {
            debug!(version = %current_version.tag, "scores cache hit");
            return Ok(scores.clone());
        }

        // Slow path: full scan and cache update
        debug!(version = %current_version.tag, "scores cache miss — full scan");
        let scored_query =
            self.table(TableName::Scores)
                .query()
                .select(lancedb::query::Select::columns(&[
                    "image_id",
                    "score",
                    "model_version",
                ]));
        let scored_batches = execute_query(&scored_query, "cached_scores:full_scan").await?;

        let scores: ScoresMap = decode_rows(&scored_batches, score_columns, decode_score_row)?
            .into_iter()
            .collect();
        info!(
            score_rows = scores.len(),
            version = %current_version.tag,
            "scores cache refreshed"
        );

        self.scores_cache
            .write()
            .await
            .cache(current_version, scores.clone());
        Ok(scores)
    }

    #[instrument(skip(self, top_scores))]
    async fn fetch_summaries_for_scores(
        &self,
        top_scores: &[(ModelScore, ImageId)],
    ) -> Result<Vec<ScoredSummary>, StoreError> {
        let score_map: HashMap<&ImageId, &ModelScore> =
            top_scores.iter().map(|(vs, id)| (id, vs)).collect();

        let query = self
            .table(TableName::Images)
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
            .only_if_expr(col_in(
                "image_id",
                top_scores.iter().map(|(_, id)| id.to_string()),
            ));
        let batches = execute_query(&query, "fetch_summaries_for_scores").await?;
        let summaries = batches_to_summaries(&batches)?;
        info!(
            summary_rows = summaries.len(),
            "read summaries for top scores"
        );

        let mut scored: Vec<ScoredSummary> = summaries
            .into_iter()
            .filter_map(|summary| {
                score_map.get(&summary.image_id).map(|vs| ScoredSummary {
                    summary,
                    scored: (*vs).clone(),
                })
            })
            .collect();

        info!(matched_rows = scored.len(), "joined scores with summaries");

        scored.sort_by(|a, b| {
            b.scored
                .score
                .partial_cmp(&a.scored.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(scored)
    }
}
