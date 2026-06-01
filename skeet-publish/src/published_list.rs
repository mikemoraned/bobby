use deadpool_redis::redis;

use crate::limit::Limit;
use crate::order::Order;
use crate::published_pair::PublishedPair;

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

    /// The redis key for this list: `{order}-{limit}`, e.g. `recency-48h`.
    pub fn name(&self) -> String {
        format!("{}-{}", self.order, self.limit)
    }

    /// The scratch key the next list is built in before being swapped into
    /// place. Never read by consumers.
    fn building_key(&self) -> String {
        format!("{}-{}:building", self.order, self.limit)
    }

    /// Atomically replace the list with `pairs`, preserving order.
    ///
    /// The new list is built in a scratch key and `RENAME`d over the target —
    /// `RENAME` is atomic, so a concurrent reader sees either the entire old
    /// list or the entire new one, never a partial write. An empty `pairs`
    /// deletes the list (an empty list and an absent key are indistinguishable
    /// to a reader).
    pub async fn replace<C>(
        &self,
        conn: &mut C,
        pairs: &[PublishedPair],
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
            return Ok(());
        }

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
        Ok(())
    }

    /// Read the full list in stored order.
    pub async fn read<C>(&self, conn: &mut C) -> Result<Vec<PublishedPair>, PublishedListError>
    where
        C: redis::aio::ConnectionLike + Send,
    {
        let raw: Vec<String> = redis::cmd("LRANGE")
            .arg(self.name())
            .arg(0)
            .arg(-1)
            .query_async(conn)
            .await?;
        raw.iter()
            .map(|s| serde_json::from_str(s).map_err(PublishedListError::from))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_order_dash_limit() {
        let list = PublishedList::new(Order::Recency, Limit::hours(48));
        assert_eq!(list.name(), "recency-48h");
    }

    #[test]
    fn name_components_parse_back() {
        let order: Order = "recency".parse().expect("order");
        let limit: Limit = "48h".parse().expect("limit");
        assert_eq!(PublishedList::new(order, limit).name(), "recency-48h");
    }

    #[test]
    fn building_key_is_distinct_from_name() {
        let list = PublishedList::new(Order::Recency, Limit::days(7));
        assert_ne!(list.building_key(), list.name());
    }
}
