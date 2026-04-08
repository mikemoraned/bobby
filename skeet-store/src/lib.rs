#![warn(clippy::all, clippy::nursery)]
mod args;
mod arrow_utils;
mod error;
pub mod health;
mod schema;
mod stored;
mod summary;
mod types;

pub use args::StoreArgs;
pub use error::StoreError;
pub use shared::{ModelVersion, Score};
pub use stored::{StoredImage, StoredImageSummary};
pub use summary::SkeetStoreSummary;
pub use types::{DiscoveredAt, ImageId, ImageRecord, InvalidImageId, OriginalAt, SkeetId, Zone};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{
    Float32Array, Int64Array, LargeBinaryArray, RecordBatch, RecordBatchIterator, StringArray,
    TimestampMicrosecondArray,
};
use chrono::Utc;
use futures::TryStreamExt;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::{CompactionOptions, OptimizeAction};
use tracing::{debug, info, instrument};

use arrow_utils::{encode_image_as_png, min_max_timestamp, typed_column};
use schema::{
    SCORE_TABLE_NAME, TABLE_NAME, VALIDATE_TABLE_NAME, images_score_v2_schema, images_v6_schema,
    validate_v1_schema,
};
use stored::{batches_to_stored_images, batches_to_summaries};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum CompactTarget {
    All,
    Images,
    Scores,
}

impl CompactTarget {
    const fn includes_images(self) -> bool {
        matches!(self, Self::All | Self::Images)
    }

    const fn includes_scores(self) -> bool {
        matches!(self, Self::All | Self::Scores)
    }
}

pub struct SkeetStore {
    images_table: lancedb::Table,
    scores_table: lancedb::Table,
    validate_table: lancedb::Table,
    writes_since_compact: AtomicU64,
    compact_every_n_writes: Option<u64>,
}

impl SkeetStore {
    #[instrument(skip(storage_options))]
    pub async fn open(
        uri: &str,
        storage_options: Vec<(String, String)>,
        compact_every_n_writes: Option<u64>,
    ) -> Result<Self, StoreError> {
        info!(uri, "opening store");
        let db = lancedb::connect(uri)
            .read_consistency_interval(Duration::ZERO)
            .storage_options(storage_options)
            .execute()
            .await?;

        let table_names = db.table_names().execute().await?;
        if !table_names.contains(&TABLE_NAME.to_string()) {
            db.create_empty_table(TABLE_NAME, images_v6_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&SCORE_TABLE_NAME.to_string()) {
            db.create_empty_table(SCORE_TABLE_NAME, images_score_v2_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&VALIDATE_TABLE_NAME.to_string()) {
            db.create_empty_table(VALIDATE_TABLE_NAME, validate_v1_schema())
                .execute()
                .await?;
        }

        let images_table = db.open_table(TABLE_NAME).execute().await?;
        let indices = images_table.list_indices().await?;
        if !indices.iter().any(|idx| idx.columns == vec!["image_id"]) {
            images_table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }

        let scores_table = db.open_table(SCORE_TABLE_NAME).execute().await?;
        let score_indices = scores_table.list_indices().await?;
        if !score_indices
            .iter()
            .any(|idx| idx.columns == vec!["image_id"])
        {
            scores_table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }
        if !score_indices
            .iter()
            .any(|idx| idx.columns == vec!["model_version"])
        {
            scores_table
                .create_index(&["model_version"], Index::Auto)
                .execute()
                .await?;
        }

        let validate_table = db.open_table(VALIDATE_TABLE_NAME).execute().await?;

        let images_stats = images_table.stats().await?;
        let scores_stats = scores_table.stats().await?;
        info!(?indices, ?images_stats, "images_table stats");
        info!(?score_indices, ?scores_stats, "scores_table stats");

        for idx in &indices {
            let stats = images_table.index_stats(&idx.name).await?;
            info!(index_name = %idx.name, ?stats, "images_table index stats");
        }
        for idx in &score_indices {
            let stats = scores_table.index_stats(&idx.name).await?;
            info!(index_name = %idx.name, ?stats, "scores_table index stats");
        }

        info!(uri, ?compact_every_n_writes, "store opened");
        Ok(Self {
            images_table,
            scores_table,
            validate_table,
            writes_since_compact: AtomicU64::new(0),
            compact_every_n_writes,
        })
    }

    #[instrument(skip(self, record), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
    pub async fn add(&self, record: &ImageRecord) -> Result<(), StoreError> {
        let schema = images_v6_schema();

        let image_bytes = encode_image_as_png(&record.image)?;
        let annotated_bytes = encode_image_as_png(&record.annotated_image)?;
        let image_id_str = record.image_id.to_string();
        let skeet_id_str = record.skeet_id.to_string();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![image_id_str.as_str()])),
                Arc::new(StringArray::from(vec![skeet_id_str.as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&image_bytes])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.discovered_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.original_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(StringArray::from(vec![record.zone.to_string().as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&annotated_bytes])),
                Arc::new(StringArray::from(vec![record.config_version.as_str()])),
                Arc::new(StringArray::from(vec![record.detected_text.as_str()])),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.images_table.add(batches).execute().await?;
        self.compact_if_needed().await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn compact(&self) -> Result<(), StoreError> {
        self.compact_table(CompactTarget::All).await
    }

    #[instrument(skip(self))]
    pub async fn compact_table(&self, target: CompactTarget) -> Result<(), StoreError> {
        // images_table stores ~2MB PNG blobs per row. Lance's compaction planner
        // treats any fragment with physical_rows < target_rows_per_fragment as a
        // candidate — see the candidacy check in plan_compaction():
        //   https://github.com/lancedb/lance/blob/v0.20.0/rust/lance/src/dataset/optimize.rs#L693
        // CompactionOptions fields documented at:
        //   https://docs.rs/lancedb/0.26.2/lancedb/table/struct.CompactionOptions.html
        //
        // With the default of 1M, all fragments are candidates and lance tries to
        // merge them into one massive fragment, requiring all image data in memory.
        //
        // Setting target_rows_per_fragment=500 means:
        //  - fragments with >500 rows are NOT candidates (left alone)
        //  - single-row fragments from add() ARE candidates, merged into ~500-row
        //    groups (~1GB), well within k8s memory limits
        //  - num_threads=1 ensures only one compaction task runs at a time
        //  - batch_size=64 limits the scanner read batch during compaction
        let images_compact_options = CompactionOptions {
            num_threads: Some(1),
            target_rows_per_fragment: 500,
            batch_size: Some(64),
            ..CompactionOptions::default()
        };
        // scores_table rows are small (image_id + f32 + model_version), so the
        // default target_rows_per_fragment (1M) is fine — memory is not a concern.
        let scores_compact_options = CompactionOptions {
            num_threads: Some(1),
            ..CompactionOptions::default()
        };

        if target.includes_images() {
            let before = self.images_table.stats().await?;
            info!(?before, "compacting images_table");
            self.images_table
                .optimize(OptimizeAction::Compact {
                    options: images_compact_options,
                    remap_options: None,
                })
                .await?;
            info!("images_table compaction done, rebuilding indices");
            self.images_table
                .optimize(OptimizeAction::Index(Default::default()))
                .await?;
            let after = self.images_table.stats().await?;
            info!(?after, "images_table optimization complete");
        }

        if target.includes_scores() {
            let before = self.scores_table.stats().await?;
            info!(?before, "compacting scores_table");
            self.scores_table
                .optimize(OptimizeAction::Compact {
                    options: scores_compact_options,
                    remap_options: None,
                })
                .await?;
            info!("scores_table compaction done, rebuilding indices");
            self.scores_table
                .optimize(OptimizeAction::Index(Default::default()))
                .await?;
            let after = self.scores_table.stats().await?;
            info!(?after, "scores_table optimization complete");
        }

        self.writes_since_compact.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub async fn storage_health(&self) -> Result<health::StoreHealth, StoreError> {
        let mut tables = Vec::new();

        for (name, table) in [
            ("images_table", &self.images_table),
            ("scores_table", &self.scores_table),
        ] {
            let stats = table.stats().await?;
            let indices = table.list_indices().await?;
            let mut index_health = Vec::new();
            for idx in &indices {
                let idx_stats = table.index_stats(&idx.name).await?;
                index_health.push(health::IndexHealth {
                    name: idx.name.clone(),
                    stats: idx_stats,
                });
            }
            tables.push(health::TableHealth {
                name: name.to_string(),
                stats,
                index_health,
            });
        }

        Ok(health::StoreHealth { tables })
    }

    async fn compact_if_needed(&self) -> Result<(), StoreError> {
        if let Some(threshold) = self.compact_every_n_writes {
            let count = self.writes_since_compact.fetch_add(1, Ordering::Relaxed) + 1;
            if count >= threshold {
                self.compact().await?;
            }
        }
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn list_all(&self) -> Result<Vec<StoredImage>, StoreError> {
        let batches: Vec<RecordBatch> =
            self.images_table.query().execute().await?.try_collect().await?;
        batches_to_stored_images(&batches)
    }

    #[instrument(skip(self))]
    pub async fn list_all_by_most_recent(&self) -> Result<Vec<StoredImage>, StoreError> {
        let mut images = self.list_all().await?;
        images.sort_by(|a, b| b.summary.discovered_at.cmp(&a.summary.discovered_at));
        Ok(images)
    }

    #[instrument(skip(self))]
    pub async fn list_all_summaries(&self) -> Result<Vec<StoredImageSummary>, StoreError> {
        let batches: Vec<RecordBatch> = self
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
            .execute()
            .await?
            .try_collect()
            .await?;
        batches_to_summaries(&batches)
    }

    #[instrument(skip(self))]
    pub async fn get_by_id(&self, image_id: &ImageId) -> Result<Option<StoredImage>, StoreError> {
        let query = self
            .images_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1);
        let plan = query.explain_plan(true).await?;
        debug!(plan, "get_by_id query plan");
        let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;
        Ok(batches_to_stored_images(&batches)?.into_iter().next())
    }

    #[instrument(skip(self))]
    pub async fn exists(&self, image_id: &ImageId) -> Result<bool, StoreError> {
        let query = self
            .images_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .select(lancedb::query::Select::columns(&["image_id"]))
            .limit(1);
        let plan = query.explain_plan(true).await?;
        debug!(plan, "exists query plan");
        let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;
        Ok(batches.iter().any(|b| b.num_rows() > 0))
    }

    #[instrument(skip(self))]
    pub async fn delete_by_id(&self, image_id: &ImageId) -> Result<(), StoreError> {
        self.images_table
            .delete(&format!("image_id = '{image_id}'"))
            .await?;
        self.compact_if_needed().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn count(&self) -> Result<usize, StoreError> {
        Ok(self.images_table.count_rows(None).await?)
    }

    #[instrument(skip(self))]
    pub async fn validate(&self) -> Result<(), StoreError> {
        let now = Utc::now();
        let timestamp_micros = now.timestamp_micros();
        let random_number = rand::random::<i64>();

        let schema = validate_v1_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(
                    TimestampMicrosecondArray::from(vec![timestamp_micros]).with_timezone("UTC"),
                ),
                Arc::new(Int64Array::from(vec![random_number])),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.validate_table.add(batches).execute().await?;

        let result_batches: Vec<RecordBatch> = self
            .validate_table
            .query()
            .only_if(format!("random_number = {random_number}"))
            .execute()
            .await?
            .try_collect()
            .await?;

        if result_batches.is_empty() {
            return Err(StoreError::ValidationFailed(
                "no rows returned for validation query".to_string(),
            ));
        }

        let timestamps =
            typed_column::<TimestampMicrosecondArray>(&result_batches[0], "timestamp")?;
        if result_batches[0].num_rows() == 0 {
            return Err(StoreError::ValidationFailed(
                "no rows returned for validation query".to_string(),
            ));
        }

        let found_micros = timestamps.value(0);
        if found_micros != timestamp_micros {
            return Err(StoreError::ValidationFailed(format!(
                "timestamp mismatch: expected {timestamp_micros}, got {found_micros}"
            )));
        }

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn unique_skeet_ids(&self) -> Result<Vec<SkeetId>, StoreError> {
        let batches: Vec<RecordBatch> = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&["skeet_id"]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for batch in &batches {
            let skeet_ids = typed_column::<StringArray>(batch, "skeet_id")?;
            for i in 0..batch.num_rows() {
                let id = skeet_ids.value(i).to_string();
                if seen.insert(id.clone()) {
                    ids.push(SkeetId::new(id)?);
                }
            }
        }

        Ok(ids)
    }

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
        let batches: Vec<RecordBatch> = self
            .scores_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

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
    pub async fn list_all_image_ids_by_most_recent(&self) -> Result<Vec<ImageId>, StoreError> {
        let batches: Vec<RecordBatch> = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "discovered_at",
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut id_times = Vec::new();
        for batch in &batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let discovered_ats =
                typed_column::<TimestampMicrosecondArray>(batch, "discovered_at")?;
            for i in 0..batch.num_rows() {
                id_times.push((image_ids.value(i).parse::<ImageId>()?, discovered_ats.value(i)));
            }
        }
        id_times.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(id_times.into_iter().map(|(id, _)| id).collect())
    }

    #[instrument(skip(self))]
    pub async fn list_unscored_image_ids_for_version(
        &self,
        model_version: &ModelVersion,
    ) -> Result<Vec<ImageId>, StoreError> {
        let all_ids = self.list_all_image_ids_by_most_recent().await?;

        let version_str = model_version.to_string();
        let scored_batches: Vec<RecordBatch> = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if(format!("model_version = '{version_str}'"))
            .execute()
            .await?
            .try_collect()
            .await?;

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

    const MAX_SCORED_SUMMARIES: usize = 100;

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
        let cutoff = Utc::now() - chrono::Duration::hours(hours as i64);
        let cutoff_us = cutoff.timestamp_micros();
        let filter = format!(
            "discovered_at >= arrow_cast({cutoff_us}, 'Timestamp(Microsecond, Some(\"UTC\"))')"
        );
        let batches: Vec<RecordBatch> = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .only_if(filter)
            .execute()
            .await?
            .try_collect()
            .await?;
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
        let scored_batches: Vec<RecordBatch> = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id", "score"]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut all_scores: Vec<(Score, ImageId)> = Vec::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let scores = typed_column::<Float32Array>(batch, "score")?;
            for i in 0..batch.num_rows() {
                let image_id: ImageId = image_ids.value(i).parse()?;
                if let Some(recent) = recent_ids
                    && !recent.contains(&image_id)
                {
                    continue;
                }
                let score = Score::new(scores.value(i))
                    .map_err(|e| StoreError::ValidationFailed(e.to_string()))?;
                all_scores.push((score, image_id));
            }
        }
        info!(score_rows = all_scores.len(), "read scores (after age filter)");

        all_scores.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_scores.truncate(limit);
        Ok(all_scores)
    }

    #[instrument(skip(self, top_scores))]
    async fn fetch_summaries_for_scores(
        &self,
        top_scores: &[(Score, ImageId)],
    ) -> Result<Vec<(StoredImageSummary, Score)>, StoreError> {
        let score_map: std::collections::HashMap<&ImageId, Score> = top_scores
            .iter()
            .map(|(s, id)| (id, *s))
            .collect();

        let in_list = top_scores
            .iter()
            .map(|(_, id)| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("image_id IN ({in_list})");

        let batches: Vec<RecordBatch> = self
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
            .only_if(filter)
            .execute()
            .await?
            .try_collect()
            .await?;
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
    ) -> Result<std::collections::HashMap<ImageId, Score>, StoreError> {
        if image_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let in_list = image_ids
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let filter = format!("image_id IN ({in_list})");

        let batches: Vec<RecordBatch> = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id", "score"]))
            .only_if(filter)
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut score_map = std::collections::HashMap::new();
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

    #[instrument(skip(self))]
    pub async fn summarise(&self) -> Result<SkeetStoreSummary, StoreError> {
        let image_count = self.images_table.count_rows(None).await?;
        let score_count = self.scores_table.count_rows(None).await?;

        let batches: Vec<RecordBatch> = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "discovered_at",
                "original_at",
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let discovered_at_range = min_max_timestamp(&batches, "discovered_at")?
            .map(|(min, max)| (DiscoveredAt::new(min), DiscoveredAt::new(max)));
        let original_at_range = min_max_timestamp(&batches, "original_at")?
            .map(|(min, max)| (OriginalAt::new(min), OriginalAt::new(max)));

        // Count distinct image_ids that have a score
        let scored_batches: Vec<RecordBatch> = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]))
            .execute()
            .await?
            .try_collect()
            .await?;

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

#[cfg(test)]
mod store_tests;
