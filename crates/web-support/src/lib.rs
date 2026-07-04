#![warn(clippy::all, clippy::nursery)]

//! Shared web helpers for Bobby's `cot` apps.

pub mod conditional_get;

pub use conditional_get::{
    http_date, not_modified_by_etag, not_modified_since, set_etag, set_last_modified,
};
