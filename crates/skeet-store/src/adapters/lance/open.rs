use std::sync::Arc;
use std::time::Duration;

use enum_map::enum_map;
use lance::dataset::ReadParams;
use lance_io::object_store::ObjectStoreParams;
use lancedb::index::Index;
use tokio::sync::RwLock;
use tracing::{info, instrument};

use super::schema::{
    TableName, images_score_v2_schema, images_v6_schema, manual_image_appraisal_v1_schema,
    manual_skeet_appraisal_v1_schema, validate_v1_schema,
};
use crate::adapters::object_store::R2MetricsWrapper;
use crate::error::StoreError;
use crate::{SkeetStore, VersionedCache};

impl SkeetStore {
    #[instrument(skip(storage_options))]
    pub async fn open(
        uri: &str,
        storage_options: Vec<(String, String)>,
        cli_name: &str,
    ) -> Result<Self, StoreError> {
        info!(uri, cli_name, "opening store");
        let db = lancedb::connect(uri)
            .read_consistency_interval(Duration::ZERO)
            .storage_options(storage_options)
            .execute()
            .await?;

        let store_wrapper = Arc::new(R2MetricsWrapper::new(
            cli_name,
            opentelemetry::global::meter("r2"),
        ));
        let read_params = ReadParams {
            store_options: Some(ObjectStoreParams {
                object_store_wrapper: Some(store_wrapper.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let table_names = db.table_names().execute().await?;
        if !table_names.contains(&TableName::Images.as_str().to_string()) {
            db.create_empty_table(TableName::Images.as_str(), images_v6_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&TableName::Scores.as_str().to_string()) {
            db.create_empty_table(TableName::Scores.as_str(), images_score_v2_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&TableName::Validate.as_str().to_string()) {
            db.create_empty_table(TableName::Validate.as_str(), validate_v1_schema())
                .execute()
                .await?;
        }
        if !table_names.contains(&TableName::SkeetAppraisal.as_str().to_string()) {
            db.create_empty_table(
                TableName::SkeetAppraisal.as_str(),
                manual_skeet_appraisal_v1_schema(),
            )
            .execute()
            .await?;
        }
        if !table_names.contains(&TableName::ImageAppraisal.as_str().to_string()) {
            db.create_empty_table(
                TableName::ImageAppraisal.as_str(),
                manual_image_appraisal_v1_schema(),
            )
            .execute()
            .await?;
        }

        let images_table = db
            .open_table(TableName::Images.as_str())
            .lance_read_params(read_params.clone())
            .execute()
            .await?;
        let indices = images_table.list_indices().await?;
        if !indices.iter().any(|idx| idx.columns == vec!["image_id"]) {
            images_table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }
        if !indices
            .iter()
            .any(|idx| idx.columns == vec!["discovered_at"])
        {
            images_table
                .create_index(&["discovered_at"], Index::Auto)
                .execute()
                .await?;
        }

        let scores_table = db
            .open_table(TableName::Scores.as_str())
            .lance_read_params(read_params.clone())
            .execute()
            .await?;
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

        let skeet_appraisal_table = db
            .open_table(TableName::SkeetAppraisal.as_str())
            .lance_read_params(read_params.clone())
            .execute()
            .await?;
        let skeet_appraisal_indices = skeet_appraisal_table.list_indices().await?;
        if !skeet_appraisal_indices
            .iter()
            .any(|idx| idx.columns == vec!["skeet_id"])
        {
            skeet_appraisal_table
                .create_index(&["skeet_id"], Index::Auto)
                .execute()
                .await?;
        }

        let image_appraisal_table = db
            .open_table(TableName::ImageAppraisal.as_str())
            .lance_read_params(read_params.clone())
            .execute()
            .await?;
        let image_appraisal_indices = image_appraisal_table.list_indices().await?;
        if !image_appraisal_indices
            .iter()
            .any(|idx| idx.columns == vec!["image_id"])
        {
            image_appraisal_table
                .create_index(&["image_id"], Index::Auto)
                .execute()
                .await?;
        }

        let validate_table = db
            .open_table(TableName::Validate.as_str())
            .lance_read_params(read_params)
            .execute()
            .await?;

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

        info!(uri, "store opened");
        let tables = enum_map! {
            TableName::Images => images_table.clone(),
            TableName::Scores => scores_table.clone(),
            TableName::Validate => validate_table.clone(),
            TableName::SkeetAppraisal => skeet_appraisal_table.clone(),
            TableName::ImageAppraisal => image_appraisal_table.clone(),
        };
        Ok(Self {
            tables,
            scores_cache: RwLock::new(VersionedCache::new()),
            store_wrapper,
        })
    }
}
