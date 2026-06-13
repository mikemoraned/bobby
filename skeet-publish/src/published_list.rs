use chrono::{DateTime, Utc};
use deadpool_redis::redis;

use crate::limit::Limit;
use crate::order::Order;
use crate::published::{PublishedImage, SCHEMA_VERSION};

/// A published redis list, identified by its `{order}-{limit}` name (e.g.
/// `recency-48h`), with the read/write helpers both the publisher and
/// `skeet-feed` use against it.
///
/// Writes replace the whole list atomically so a concurrent reader never
/// observes a half-written list (see [`PublishedList::replace`]).
pub struct PublishedList {
    order: Order,
    limit: Limit,
}

#[derive(Debug, thiserror::Error)]
pub enum PublishedListError {
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl PublishedList {
    pub const fn new(order: Order, limit: Limit) -> Self {
        Self { order, limit }
    }

    /// The redis key for this list: `{version}-{order}-{limit}`, e.g.
    /// `v3-recency-48h`. The schema version prefixes the name so an
    /// incompatible `PublishedImage` change doesn't collide with an old reader/writer
    /// (see [`SCHEMA_VERSION`]).
    pub fn name(&self) -> String {
        format!("{SCHEMA_VERSION}-{}-{}", self.order, self.limit)
    }

    /// The scratch key the next list is built in before being swapped into
    /// place. Never read by consumers.
    fn building_key(&self) -> String {
        format!("{}:building", self.name())
    }

    /// The companion key holding when the list was last published (RFC 3339).
    fn refreshed_at_key(&self) -> String {
        format!("{}:refreshed-at", self.name())
    }

    /// Atomically replace the list with `pairs` (preserving order) and record
    /// `refreshed_at` as the publish time.
    ///
    /// The new list is built in a scratch key and `RENAME`d over the target —
    /// `RENAME` is atomic, so a concurrent reader sees either the entire old
    /// list or the entire new one, never a partial write. An empty `pairs`
    /// deletes the list (an empty list and an absent key are indistinguishable
    /// to a reader). The `refreshed-at` companion key is written last, so a
    /// reader that races the swap sees an unchanged-or-older timestamp, never a
    /// newer one paired with an old list.
    pub async fn replace<C>(
        &self,
        conn: &mut C,
        pairs: &[PublishedImage],
        refreshed_at: DateTime<Utc>,
    ) -> Result<(), PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let name = self.name();
        let building = self.building_key();

        // Clear any scratch key left by an interrupted previous run.
        redis::cmd("DEL").arg(&building).exec_async(conn).await?;

        if pairs.is_empty() {
            redis::cmd("DEL").arg(&name).exec_async(conn).await?;
        } else {
            let encoded = pairs
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()?;
            redis::cmd("RPUSH")
                .arg(&building)
                .arg(&encoded)
                .exec_async(conn)
                .await?;
            redis::cmd("RENAME")
                .arg(&building)
                .arg(&name)
                .exec_async(conn)
                .await?;
        }

        redis::cmd("SET")
            .arg(self.refreshed_at_key())
            .arg(refreshed_at.to_rfc3339())
            .exec_async(conn)
            .await?;
        Ok(())
    }

    /// When the list was last published, if it has been.
    pub async fn refreshed_at<C>(
        &self,
        conn: &mut C,
    ) -> Result<Option<DateTime<Utc>>, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let raw: Option<String> = redis::cmd("GET")
            .arg(self.refreshed_at_key())
            .query_async(conn)
            .await?;
        Ok(decode_refreshed_at(raw))
    }

    /// Read the full list in stored order.
    pub async fn read<C>(&self, conn: &mut C) -> Result<Vec<PublishedImage>, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let raw: Vec<String> = redis::cmd("LRANGE")
            .arg(self.name())
            .arg(0)
            .arg(-1)
            .query_async(conn)
            .await?;
        decode_images(raw)
    }

    /// Read the list and its publish time together in a single round-trip
    /// (`LRANGE` + `GET` pipelined) — the pair every consumer reads at once.
    pub async fn read_with_refreshed_at<C>(
        &self,
        conn: &mut C,
    ) -> Result<(Vec<PublishedImage>, Option<DateTime<Utc>>), PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let (raw_list, raw_refreshed): (Vec<String>, Option<String>) = redis::pipe()
            .cmd("LRANGE")
            .arg(self.name())
            .arg(0)
            .arg(-1)
            .cmd("GET")
            .arg(self.refreshed_at_key())
            .query_async(conn)
            .await?;
        Ok((decode_images(raw_list)?, decode_refreshed_at(raw_refreshed)))
    }

    /// Whether the list key currently exists in redis. Note an empty list has no
    /// key (`replace` deletes it), so this is `false` both when the list was
    /// never published and when its content is genuinely empty.
    pub async fn exists<C>(&self, conn: &mut C) -> Result<bool, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let present: bool = redis::cmd("EXISTS")
            .arg(self.name())
            .query_async(conn)
            .await?;
        Ok(present)
    }
}

/// Decode the JSON-encoded list entries read back from redis.
fn decode_images(raw: Vec<String>) -> Result<Vec<PublishedImage>, PublishedListError> {
    raw.iter()
        .map(|s| serde_json::from_str(s).map_err(PublishedListError::from))
        .collect()
}

/// Decode the RFC 3339 `refreshed-at` value, treating an unparseable one as absent.
fn decode_refreshed_at(raw: Option<String>) -> Option<DateTime<Utc>> {
    raw.and_then(|s| {
        DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_version_order_limit() {
        let list = PublishedList::new(Order::Recency, Limit::hours(48));
        assert_eq!(list.name(), "v3-recency-48h");
    }

    #[test]
    fn name_components_parse_back() {
        let order: Order = "recency".parse().expect("order");
        let limit: Limit = "48h".parse().expect("limit");
        assert_eq!(PublishedList::new(order, limit).name(), "v3-recency-48h");
    }

    #[test]
    fn building_key_is_distinct_from_name() {
        let list = PublishedList::new(Order::Recency, Limit::days(7));
        assert_ne!(list.building_key(), list.name());
    }
}
