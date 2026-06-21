//! Boundary data types owned by the store: the entities that flow across its
//! ports.
//!
//! Pure data with `shared`-only dependencies — no LanceDB or Arrow. The adapter
//! (`lance`) decodes storage rows into these; the ports trade in them. Pure
//! cross-crate data types like `Appraisal`, `DiscoveredAt`, and `OriginalAt`
//! live in `shared` and are imported from there directly, not re-exported.

mod image;
mod score;
mod version;

pub use image::{ImageRecord, StoredImage, StoredImageSummary, StoredOriginal};
pub use score::ScoresMap;
pub use version::Version;
