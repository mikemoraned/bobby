use deadpool_redis::redis;

use crate::published::SCHEMA_VERSION;
use crate::published_list::{PublishedList, PublishedListError};

/// The set of published lists the publisher currently writes, stored in redis so
/// a consumer (e.g. `skeet-appraise`) can discover the available feeds instead of
/// being told them via config.
///
/// Held as a redis SET of [`PublishedList::name`] keys (e.g. `v3-quality-4w`)
/// under a single schema-versioned key. It is a set, not an ordered list:
/// consumers impose their own display order, so membership is all that's recorded.
pub struct PublishedListCatalog;

impl PublishedListCatalog {
    /// The redis key holding the catalog set: `{version}-feed-catalog`, e.g.
    /// `v3-feed-catalog`. The schema version matches the published-list keys so a
    /// schema bump retires the catalog alongside the lists it describes.
    pub fn key() -> String {
        format!("{SCHEMA_VERSION}-feed-catalog")
    }

    /// Replace the catalog with exactly `lists`. Empty `lists` deletes the key.
    pub async fn write<C>(conn: &mut C, lists: &[PublishedList]) -> Result<(), PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let key = Self::key();
        let mut pipe = redis::pipe();
        pipe.cmd("DEL").arg(&key).ignore();
        if !lists.is_empty() {
            let names: Vec<String> = lists.iter().map(PublishedList::name).collect();
            pipe.cmd("SADD").arg(&key).arg(&names).ignore();
        }
        pipe.exec_async(conn).await?;
        Ok(())
    }

    /// Read the catalog, dropping any member that doesn't parse as a published-list
    /// key (so a stray or future-schema entry can't fail discovery).
    pub async fn read<C>(conn: &mut C) -> Result<Vec<PublishedList>, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let names: Vec<String> = redis::cmd("SMEMBERS")
            .arg(Self::key())
            .query_async(conn)
            .await?;
        Ok(names
            .iter()
            .filter_map(|n| PublishedList::from_name(n).ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_versioned() {
        assert_eq!(PublishedListCatalog::key(), "v3-feed-catalog");
    }
}
