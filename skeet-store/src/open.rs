use std::sync::Arc;
use std::time::Duration;

use lance::dataset::ReadParams;
use lance_io::object_store::ObjectStoreParams;
use lancedb::index::Index;
use tokio::sync::RwLock;
use tracing::{info, instrument};

use crate::error::StoreError;
use crate::r2_metrics::R2MetricsWrapper;
use crate::schema::{
    IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME, TABLE_NAME,
    VALIDATE_TABLE_NAME, images_score_v2_schema, images_v6_schema,
    manual_image_appraisal_v1_schema, manual_skeet_appraisal_v1_schema, validate_v1_schema,
};
use crate::SkeetStore;

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
        if !table_names.contains(&SKEET_APPRAISAL_TABLE_NAME.to_string()) {
            db.create_empty_table(
                SKEET_APPRAISAL_TABLE_NAME,
                manual_skeet_appraisal_v1_schema(),
            )
            .execute()
            .await?;
        }
        if !table_names.contains(&IMAGE_APPRAISAL_TABLE_NAME.to_string()) {
            db.create_empty_table(
                IMAGE_APPRAISAL_TABLE_NAME,
                manual_image_appraisal_v1_schema(),
            )
            .execute()
            .await?;
        }

        let images_table = db
            .open_table(TABLE_NAME)
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
            .open_table(SCORE_TABLE_NAME)
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
            .open_table(SKEET_APPRAISAL_TABLE_NAME)
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
            .open_table(IMAGE_APPRAISAL_TABLE_NAME)
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
            .open_table(VALIDATE_TABLE_NAME)
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
        let tables = vec![
            (TABLE_NAME, images_table.clone()),
            (SCORE_TABLE_NAME, scores_table.clone()),
            (SKEET_APPRAISAL_TABLE_NAME, skeet_appraisal_table.clone()),
            (IMAGE_APPRAISAL_TABLE_NAME, image_appraisal_table.clone()),
            (VALIDATE_TABLE_NAME, validate_table.clone()),
        ];
        Ok(Self {
            images_table,
            scores_table,
            validate_table,
            skeet_appraisal_table,
            image_appraisal_table,
            tables,
            scores_cache: RwLock::new(None),
            store_wrapper: Some(store_wrapper),
        })
    }
}
