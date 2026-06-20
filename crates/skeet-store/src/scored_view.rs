use std::collections::{HashMap, HashSet};

use arrow_array::{Float32Array, StringArray};
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::ImageId;
use tracing::{debug, info, instrument};

use crate::arrow_utils::{min_max_timestamp, typed_column};
use crate::images::Images;
use crate::lancedb_utils::execute_query;
use crate::schema::SCORE_TABLE_NAME;
use crate::stored::batches_to_summaries;
use crate::types::{DiscoveredAt, OriginalAt};
use crate::version::TableVersions;
use crate::{ModelVersion, Score, SkeetStore, SkeetStoreSummary, StoreError, StoredImageSummary};

/// The full scores table, keyed by image id — the value cached by
/// `cached_scores`, gated on the scores table version.
pub type ScoresMap = HashMap<ImageId, (Score, ModelVersion)>;

/// Cross-table read-models that join the images and scores tables.
///
/// These — the scored feed views, the unscored backlog, and the store summary —
/// belong to neither the [`crate::Images`] nor [`crate::Scores`] port because
/// each one reads both tables.
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
    async fn summarise(&self) -> Result<SkeetStoreSummary, StoreError>;
}

#[async_trait]
impl ScoredView for SkeetStore {
    #[instrument(skip(self))]
    async fn list_unscored_image_ids(
        &self,
        since: Option<&DiscoveredAt>,
    ) -> Result<Vec<ImageId>, StoreError> {
        let all_ids = self.list_all_image_ids_by_most_recent(since).await?;

        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]));
        let scored_batches = execute_query(&scored_query, "list_unscored:scored_ids").await?;

        let mut scored = std::collections::HashSet::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                scored.insert(image_ids.value(i).to_string());
            }
        }

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
    ) -> Result<Vec<(StoredImageSummary, Score, ModelVersion)>, StoreError> {
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
    ) -> Result<Vec<(StoredImageSummary, Score, ModelVersion)>, StoreError> {
        let windowed_ids = self.find_image_ids_published_since(cutoff).await?;
        if windowed_ids.is_empty() {
            return Ok(Vec::new());
        }

        let all_scores = self.cached_scores().await?;
        let scored: Vec<(Score, ModelVersion, ImageId)> = windowed_ids
            .iter()
            .filter_map(|id| {
                all_scores
                    .get(id)
                    .filter(|(_, mv)| known_versions.contains(mv))
                    .map(|(score, mv)| (*score, mv.clone(), id.clone()))
            })
            .collect();
        if scored.is_empty() {
            return Ok(Vec::new());
        }

        self.fetch_summaries_for_scores(&scored).await
    }

    #[instrument(skip(self))]
    async fn summarise(&self) -> Result<SkeetStoreSummary, StoreError> {
        let image_count = self.images_table.count_rows(None).await?;
        let score_count = self.scores_table.count_rows(None).await?;

        let timestamps_query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "discovered_at",
                "original_at",
            ]));
        let batches = execute_query(&timestamps_query, "summarise:timestamps").await?;

        let discovered_at_range = min_max_timestamp(&batches, "discovered_at")?
            .map(|(min, max)| (DiscoveredAt::new(min), DiscoveredAt::new(max)));
        let original_at_range = min_max_timestamp(&batches, "original_at")?
            .map(|(min, max)| (OriginalAt::new(min), OriginalAt::new(max)));

        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]));
        let scored_batches = execute_query(&scored_query, "summarise:scored_ids").await?;

        let mut scored_ids = std::collections::HashSet::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                scored_ids.insert(image_ids.value(i).to_string());
            }
        }

        Ok(SkeetStoreSummary {
            image_count,
            score_count,
            scored_image_count: scored_ids.len(),
            discovered_at_range,
            original_at_range,
        })
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
        let cutoff_us = cutoff.timestamp_micros();
        let filter = format!(
            "original_at >= arrow_cast({cutoff_us}, 'Timestamp(Microsecond, Some(\"UTC\"))')"
        );
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if(filter);
        let batches = execute_query(&query, "find_image_ids_published_since").await?;
        let mut ids = HashSet::new();
        for batch in &batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                let image_id: ImageId = image_ids.value(i).parse()?;
                ids.insert(image_id);
            }
        }
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

    #[instrument(skip(self, recent_ids, known_versions))]
    async fn read_top_scores(
        &self,
        limit: usize,
        recent_ids: &Option<std::collections::HashSet<ImageId>>,
        known_versions: &HashSet<ModelVersion>,
    ) -> Result<Vec<(Score, ModelVersion, ImageId)>, StoreError> {
        let all_scores_map = self.cached_scores().await?;

        let mut all_scores: Vec<(Score, ModelVersion, ImageId)> = all_scores_map
            .iter()
            .filter(|(id, _)| recent_ids.as_ref().is_none_or(|recent| recent.contains(id)))
            .filter(|(_, (_, mv))| known_versions.contains(mv))
            .map(|(id, (score, mv))| (*score, mv.clone(), id.clone()))
            .collect();
        info!(
            score_rows = all_scores.len(),
            "read scores (after age filter)"
        );

        all_scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        all_scores.truncate(limit);
        Ok(all_scores)
    }

    #[instrument(skip(self))]
    async fn cached_scores(&self) -> Result<ScoresMap, StoreError> {
        let current_version = self.table_version(SCORE_TABLE_NAME).await?;

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
        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "score",
                "model_version",
            ]));
        let scored_batches = execute_query(&scored_query, "cached_scores:full_scan").await?;

        let mut scores = HashMap::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let score_vals = typed_column::<Float32Array>(batch, "score")?;
            let model_versions = typed_column::<StringArray>(batch, "model_version")?;
            for i in 0..batch.num_rows() {
                let image_id: ImageId = image_ids.value(i).parse()?;
                let score = Score::new(score_vals.value(i))?;
                let model_version = ModelVersion::from(model_versions.value(i));
                scores.insert(image_id, (score, model_version));
            }
        }
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
        top_scores: &[(Score, ModelVersion, ImageId)],
    ) -> Result<Vec<(StoredImageSummary, Score, ModelVersion)>, StoreError> {
        let score_map: HashMap<&ImageId, (Score, ModelVersion)> = top_scores
            .iter()
            .map(|(s, mv, id)| (id, (*s, mv.clone())))
            .collect();

        let in_list = top_scores
            .iter()
            .map(|(_, _, id)| format!("'{id}'"))
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
        info!(
            summary_rows = summaries.len(),
            "read summaries for top scores"
        );

        let mut scored: Vec<(StoredImageSummary, Score, ModelVersion)> = summaries
            .into_iter()
            .filter_map(|s| {
                let entry = score_map.get(&s.image_id).cloned();
                entry.map(|(sc, mv)| (s, sc, mv))
            })
            .collect();

        info!(matched_rows = scored.len(), "joined scores with summaries");

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored)
    }
}
