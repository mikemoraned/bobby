use std::time::Duration;

use deadpool_redis::redis::{AsyncConnectionConfig, Client, RedisError, aio::MultiplexedConnection};

/// redis's defaults (1s connect, 500ms response) are far too tight for a remote
/// Upstash TLS endpoint reached over the public internet; use generous timeouts
/// that still bound a genuine hang.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Open a fresh multiplexed connection to the publish server.
///
/// A `rediss://` URL negotiates TLS via rustls (the `tokio-rustls-comp` feature
/// on `deadpool-redis` feature-unifies TLS onto `redis`); the caller must have
/// installed a rustls crypto provider first. We connect fresh per publish cycle
/// / read rather than pooling, so an idle Upstash drop never leaves a stale
/// connection behind — and so we can override redis's too-tight default
/// timeouts, which a pool gives no way to set.
pub async fn connect(url: &str) -> Result<MultiplexedConnection, RedisError> {
    let client = Client::open(url)?;
    let config = AsyncConnectionConfig::new()
        .set_connection_timeout(Some(TIMEOUT))
        .set_response_timeout(Some(TIMEOUT));
    client
        .get_multiplexed_async_connection_with_config(&config)
        .await
}
