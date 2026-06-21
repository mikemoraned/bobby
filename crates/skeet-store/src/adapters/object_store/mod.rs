//! The R2/SSE-C object-store layer the `lance` adapter writes through.
//!
//! A separable sibling of the adapter, tied to the external R2 deployment: the
//! connection/encryption configuration (`args`) and the operation-counting
//! wrapper (`r2_metrics`). The future image-encryption codec belongs here too.

mod args;
mod r2_metrics;

pub use args::StoreArgs;
pub use r2_metrics::R2MetricsWrapper;
