//! Persistence for pruned images, refine scores, and manual appraisals.
//!
//! Laid out as ports & adapter (deps point inward: `model` ← `ports` ← `lance`
//! → `object_store`). `ports/` holds the storage-agnostic traits consumers bind
//! to; `lance/` is the single concrete LanceDB/R2 adapter implementing them, with
//! all Arrow/LanceDB detail kept private to it. See
//! `docs/skeet-store-architecture.md`.
#![warn(clippy::all, clippy::nursery)]
mod error;
pub mod health;
mod lance;
mod model;
mod object_store;
mod observability;
mod ports;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_utils;
pub mod versioned_cache;

pub use error::StoreError;
pub use lance::{
    Appraisals, IMAGE_APPRAISAL_TABLE_NAME, SCORE_TABLE_NAME, SKEET_APPRAISAL_TABLE_NAME,
    SkeetStore, TABLE_NAME, VALIDATE_TABLE_NAME,
};
pub use model::{ImageRecord, StoredImage, StoredImageSummary, StoredOriginal, Version};
pub use object_store::StoreArgs;
pub use observability::StoreMetrics;
pub use ports::{AppraisalSource, Images, ScoredView, Scores, TableVersions};
pub use shared::{Appraiser, Band, ImageId, ModelVersion, Score};
pub use versioned_cache::VersionedCache;

#[cfg(test)]
mod store_tests;
