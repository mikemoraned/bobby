#![warn(clippy::all, clippy::nursery)]

pub mod handlers;

use std::sync::OnceLock;

use skeet_store::StoreArgs;

pub static STORE_ARGS: OnceLock<StoreArgs> = OnceLock::new();
