use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, TimeUnit};

pub const TABLE_NAME: &str = "images_v6";
pub const VALIDATE_TABLE_NAME: &str = "validate_v1";

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
