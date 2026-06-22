use std::fmt;
use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use enum_map::Enum;

/// The logical tables the store manages — the single source of truth for table
/// identity.
///
/// Keying the [`SkeetStore`](crate::SkeetStore) table handles by this enum (via
/// an `EnumMap`) makes "do this for every table" total and exhaustive: adding a
/// variant is a compile error until every `match` and the `enum_map!`
/// constructor handle it. [`as_str`](Self::as_str) yields the on-disk name for
/// the string-keyed [`TableVersions`](crate::TableVersions) port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Enum)]
pub enum TableName {
    Images,
    Scores,
    Validate,
    SkeetAppraisal,
    ImageAppraisal,
}

impl TableName {
    /// The on-disk LanceDB table name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Images => "images_v6",
            Self::Scores => "images_score_v2",
            Self::Validate => "validate_v1",
            Self::SkeetAppraisal => "manual_skeet_appraisal_v1",
            Self::ImageAppraisal => "manual_image_appraisal_v1",
        }
    }

    /// Resolve an on-disk table name back to its variant. `None` for any name the
    /// store doesn't manage — the inverse of [`as_str`](Self::as_str).
    pub fn from_name(name: &str) -> Option<Self> {
        (0..Self::LENGTH)
            .map(Self::from_usize)
            .find(|t| t.as_str() == name)
    }
}

impl fmt::Display for TableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

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

pub fn appraisal_schema(id_column: &str) -> Arc<Schema> {
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
        // Backs the Rust `Zone` field; named `archetype` on disk for historical
        // reasons (rename deferred to a future `images_v7`).
        Field::new("archetype", DataType::Utf8, false),
        Field::new("annotated_image", DataType::LargeBinary, false),
        Field::new("config_version", DataType::Utf8, false),
        Field::new("detected_text", DataType::Utf8, false),
    ]))
}
