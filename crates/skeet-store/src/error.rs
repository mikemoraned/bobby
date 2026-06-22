#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("LanceDB error: {0}")]
    Lance(#[from] lancedb::Error),

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),

    #[error("Image encoding error: {0}")]
    ImageEncoding(#[from] image::ImageError),

    #[error("column '{column}' missing or has unexpected type")]
    ColumnTypeMismatch { column: String },

    #[error("invalid image_id in store: {0}")]
    InvalidImageId(#[from] shared::InvalidImageId),

    #[error("invalid skeet_id in store: {0}")]
    InvalidSkeetId(#[from] shared::skeet_id::SkeetIdError),

    #[error("invalid zone in store: {0}")]
    InvalidZone(#[from] shared::ParseZoneError),

    #[error("invalid band in store: {0}")]
    InvalidBand(#[from] shared::ParseBandError),

    #[error("invalid appraiser in store: {0}")]
    InvalidAppraiser(#[from] shared::ParseAppraiserError),

    #[error("invalid score in store: {0}")]
    InvalidScore(#[from] shared::InvalidScore),

    #[error("storage validation failed: {0}")]
    ValidationFailed(String),

    #[error("limit {requested} exceeds maximum {maximum}")]
    LimitExceeded { requested: usize, maximum: usize },

    #[error("cannot get fragment count for table '{table}': {reason}")]
    CannotGetFragmentCount { table: String, reason: String },

    #[error("unknown table: {0}")]
    UnknownTable(String),
}
