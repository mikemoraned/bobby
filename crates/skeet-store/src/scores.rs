use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_array::{Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::ImageId;
use tracing::{debug, info, instrument};

use crate::arrow_utils::typed_column;
use crate::images::Images;
use crate::lancedb_utils::execute_query;
use crate::schema::{SCORE_TABLE_NAME, images_score_v2_schema};
use crate::stored::batches_to_summaries;
use crate::version::TableVersions;
use crate::{ModelVersion, Score, SkeetStore, StoreError, StoredImageSummary};

/// The full scores table, keyed by image id — the value cached by
/// `cached_scores`, gated on the scores table version.
pub type ScoresMap = HashMap<ImageId, (Score, ModelVersion)>;

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

impl SkeetStore {
    const MAX_SCORED_SUMMARIES: usize = 100;

    /// Image IDs that have no score — regardless of which
    /// `model_version` produced any existing score.
    #[instrument(skip(self))]
    pub async fn list_unscored_image_ids(
        &self,
        since: Option<&crate::DiscoveredAt>,
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

    /// Top scored summaries, considering only scores whose `model_version` is in
    /// `known_versions`. Unknown versions (e.g. written by an unregistered staging
    /// model) are discarded at read time — see `docs/versioning.md`.
    #[instrument(skip(self, known_versions))]
    pub async fn list_scored_summaries_by_score(
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

    /// All scored summaries whose skeet was published (`original_at`) at or after
    /// `cutoff` and whose `model_version` is in `known_versions`, **uncapped** and
    /// in no particular order (the caller orders).
    ///
    /// Unlike [`Self::list_scored_summaries_by_score`] there is no top-N-by-score
    /// truncation: every scored image in the recency window is returned, so a
    /// recent low-score image is never dropped. Windowing is on publish time
    /// (`original_at`), not discovery time. Scores from unregistered model
    /// versions are discarded at read time — see `docs/versioning.md`.
    #[instrument(skip(self, known_versions))]
    pub async fn list_scored_summaries_published_since(
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
