use deadpool_redis::redis;

use crate::published::SCHEMA_VERSION;
use crate::published_list::PublishedListError;

/// Redis accessor for the precalculated "images examined" count.
///
/// Stored as a plain integer under a single schema-versioned key
/// (`{version}-examined-count`) so a reader and writer at different schema
/// versions never collide (see [`SCHEMA_VERSION`]).
pub struct ExaminedCount;

impl ExaminedCount {
    /// The redis key holding the count: `{version}-examined-count`.
    fn key() -> String {
        format!("{SCHEMA_VERSION}-examined-count")
    }

    /// Overwrite the stored count.
    pub async fn write<C>(conn: &mut C, count: u64) -> Result<(), PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        redis::cmd("SET")
            .arg(Self::key())
            .arg(count)
            .exec_async(conn)
            .await?;
        Ok(())
    }

    /// Read the stored count, or `None` if it has never been published — a reader
    /// races a never-written key during a fresh deploy, so absence is expected
    /// and not an error.
    pub async fn read<C>(conn: &mut C) -> Result<Option<u64>, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let count: Option<u64> = redis::cmd("GET").arg(Self::key()).query_async(conn).await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_version_prefixed() {
        assert_eq!(ExaminedCount::key(), "v3-examined-count");
    }
}
