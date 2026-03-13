use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("invalid path: {0}")]
    InvalidPath(PathBuf),

    #[error("LanceDB error: {0}")]
    Lance(#[from] lancedb::Error),

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),

    #[error("Image encoding error: {0}")]
    ImageEncoding(#[from] image::ImageError),

    #[error("column '{column}' missing or has unexpected type")]
    ColumnTypeMismatch { column: String },

    #[error("invalid image_id in store: {0}")]
    InvalidImageId(#[from] uuid::Error),

    #[error("invalid archetype in store: {0}")]
    InvalidArchetype(String),
}
