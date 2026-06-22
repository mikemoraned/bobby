use std::collections::HashSet;

use async_trait::async_trait;

use crate::Version;
use crate::error::StoreError;

/// Store-agnostic source of per-table version tokens.
///
/// Splits the *source* of a version token from the version-gated lazy-refresh
/// *mechanism* in [`crate::VersionedCache`]: callers gate on opaque
/// [`Version`] tags rather than a LanceDB version counter, so the freshness
/// logic no longer depends on the storage backend.
#[async_trait]
pub trait TableVersions: Send + Sync {
    /// The current version token for a single logical table. `Version.tag` is an
    /// opaque string, so two tokens compare equal iff the table has not changed
    /// between the calls — the comparable key a [`crate::VersionedCache`] gates on.
    async fn table_version(&self, table: &str) -> Result<Version, StoreError>;

    /// Snapshot the version token of every table at once.
    async fn version_snapshot(&self) -> Result<HashSet<Version>, StoreError>;
}
