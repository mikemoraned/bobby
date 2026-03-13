#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("LanceDB error: {0}")]
    Lance(#[from] lancedb::Error),

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),

    #[error("Image encoding error: {0}")]
    ImageEncoding(#[from] image::ImageError),
}
