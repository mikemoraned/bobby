//! Adapter implementations of the store's ports.
//!
//! `lance` is the concrete LanceDB/R2 adapter (the one implementation of every
//! port today); `object_store` is the R2/SSE-C layer it writes through. Grouping
//! them here keeps the ports-and-adapters split visible in the file tree:
//! everything storage-specific lives under `adapters/`, so `ports`/`model` stay
//! free of LanceDB and Arrow.

pub mod lance;
pub mod object_store;
