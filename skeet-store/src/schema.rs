use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, TimeUnit};

pub const TABLE_NAME: &str = "images_v3";

pub fn images_v3_schema() -> Arc<Schema> {
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
    ]))
}
