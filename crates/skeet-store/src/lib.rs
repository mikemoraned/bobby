//! Persistence for pruned images, refine scores, and manual appraisals.
//!
//! Laid out as ports & adapters (deps point inward: `model` ← `ports` ←
//! `adapters`). `ports/` holds the storage-agnostic traits consumers bind to;
//! `adapters/lance` is the single concrete LanceDB/R2 adapter implementing them,
//! with all Arrow/LanceDB detail kept private to it. See
//! `docs/skeet-store-architecture.md`.
#![warn(clippy::all, clippy::nursery)]
mod adapters;
mod error;
pub mod health;
mod model;
mod observability;
mod ports;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_utils;
pub mod versioned_cache;

pub use adapters::lance::{SkeetStore, TableName};
pub use adapters::object_store::StoreArgs;
pub use error::StoreError;
pub use model::{
    ImageRecord, ModelScore, ScoredSummary, StoredImage, StoredImageSummary, StoredOriginal,
    Version,
};
pub use observability::StoreMetrics;
pub use ports::{Appraisals, AppraisalsSource, Images, ScoredView, Scores, TableVersions};
pub use shared::{Appraiser, Band, ImageId, ModelVersion, Score};
pub use versioned_cache::VersionedCache;

#[cfg(test)]
mod store_tests;
