use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, TimeUnit};

pub const TABLE_NAME: &str = "images_v6";
pub const SCORE_TABLE_NAME: &str = "images_score_v2";
pub const VALIDATE_TABLE_NAME: &str = "validate_v1";
pub const SKEET_APPRAISAL_TABLE_NAME: &str = "manual_skeet_appraisal_v1";
pub const IMAGE_APPRAISAL_TABLE_NAME: &str = "manual_image_appraisal_v1";

pub fn validate_v1_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("random_number", DataType::Int64, false),
    ]))
}

pub fn images_score_v2_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("image_id", DataType::Utf8, false),
        Field::new("score", DataType::Float32, false),
        Field::new("model_version", DataType::Utf8, false),
    ]))
}

fn appraisal_schema(id_column: &str) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(id_column, DataType::Utf8, false),
        Field::new("band", DataType::Utf8, false),
        Field::new("appraiser", DataType::Utf8, false),
        Field::new(
            "appraised_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
    ]))
}

pub fn manual_skeet_appraisal_v1_schema() -> Arc<Schema> {
    appraisal_schema("skeet_id")
}

pub fn manual_image_appraisal_v1_schema() -> Arc<Schema> {
    appraisal_schema("image_id")
}

pub fn images_v6_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("image_id", DataType::Utf8, false),
        Field::new("skeet_id", DataType::Utf8, false),
        Field::new("image", DataType::LargeBinary, false),
        Field::new(
            "discovered_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new(
            "original_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("archetype", DataType::Utf8, false),
        Field::new("annotated_image", DataType::LargeBinary, false),
        Field::new("config_version", DataType::Utf8, false),
        Field::new("detected_text", DataType::Utf8, false),
    ]))
}
