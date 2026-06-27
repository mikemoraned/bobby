//! Boundary data types owned by the store: the entities that flow across its
//! ports.
//!
//! Pure data with `shared`-only dependencies — no LanceDB or Arrow. The adapter
//! (`lance`) decodes storage rows into these; the ports trade in them. Pure
//! cross-crate data types like `Appraisal`, `DiscoveredAt`, and `OriginalAt`
//! live in `shared` and are imported from there directly, not re-exported.

mod image;
mod prune_stats;
mod score;
mod version;

pub use image::{ImageRecord, StoredImage, StoredImageSummary, StoredOriginal};
pub use prune_stats::PruneStats;
pub use score::{ModelScore, ScoredSummary, ScoresMap};
pub use version::Version;
