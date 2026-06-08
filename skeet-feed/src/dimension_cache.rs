use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use image::ImageReader;
use tower::{Layer, Service};
use tracing::{debug, warn};

/// An image's pixel dimensions.
#[derive(Clone, Copy)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// Max images fetched concurrently during a [`DimensionCache::prefetch`].
const PREFETCH_CONCURRENCY: usize = 16;

/// A never-stale, single-write cache of image dimensions keyed by CDN url.
///
/// A render asks for an image's dimensions via [`DimensionCache::dimensions`]: a
/// hit returns immediately; a miss blocks on a GET of the image and reads its
/// header, then caches the result. Blocking is acceptable because the home page
/// is loaded rarely; [`DimensionCache::prefetch`] warms the cache at startup so
/// the live per-render cost is only for images discovered since boot.
///
/// Dimensions for a CDN url never change, so entries are written once and never
/// invalidated. The cache is in-memory; it survives Fly suspend/resume (memory
/// is snapshotted) and is only lost on a deploy/restart, after which the startup
/// prefetch refills it.
pub struct DimensionCache {
    /// `None` disables fetching: lookups then only return preset entries (used
    /// by tests to stay off the network).
    client: Option<reqwest::Client>,
    entries: Arc<RwLock<HashMap<String, Dimensions>>>,
}

impl DimensionCache {
    /// A cache that fetches-on-miss over the network.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client: Some(client),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// A cache that never fetches — only returns entries added via
    /// [`DimensionCache::preset`]. For tests, so they don't touch the network.
    pub fn cache_only() -> Self {
        Self {
            client: None,
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Insert a known dimension directly (test seeding / warm start).
    pub fn preset(&self, url: impl Into<String>, width: u32, height: u32) {
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(url.into(), Dimensions { width, height });
        }
    }

    fn cached(&self, url: &str) -> Option<Dimensions> {
        self.entries.read().ok()?.get(url).copied()
    }

    fn store(&self, url: String, dims: Dimensions) {
        if let Ok(mut entries) = self.entries.write() {
            entries.insert(url, dims);
        }
    }

    /// Dimensions for `url`: a cached hit returns immediately; a miss blocks on a
    /// fetch (when fetching is enabled), caches the result, and returns it.
    /// `None` if the image can't be fetched/decoded — the caller then renders
    /// without an explicit aspect ratio.
    pub async fn dimensions(&self, url: &str) -> Option<Dimensions> {
        if let Some(dims) = self.cached(url) {
            return Some(dims);
        }
        let client = self.client.as_ref()?;
        let dims = fetch_dimensions(client, url).await?;
        self.store(url.to_string(), dims);
        Some(dims)
    }

    /// Warm the cache for `urls` (skipping ones already cached) with bounded
    /// concurrency, blocking until all have been attempted. Failures are ignored
    /// (those images fall back to a lazy fetch on render).
    pub async fn prefetch(&self, urls: impl IntoIterator<Item = String>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let mut pending = urls
            .into_iter()
            .filter(|url| self.cached(url).is_none())
            .collect::<Vec<_>>()
            .into_iter();

        let mut in_flight = tokio::task::JoinSet::new();
        let spawn = |set: &mut tokio::task::JoinSet<(String, Option<Dimensions>)>, url: String| {
            let client = client.clone();
            set.spawn(async move {
                let dims = fetch_dimensions(&client, &url).await;
                (url, dims)
            });
        };

        for url in pending.by_ref().take(PREFETCH_CONCURRENCY) {
            spawn(&mut in_flight, url);
        }
        while let Some(result) = in_flight.join_next().await {
            if let Ok((url, Some(dims))) = result {
                self.store(url, dims);
            }
            if let Some(url) = pending.next() {
                spawn(&mut in_flight, url);
            }
        }
    }
}

impl Default for DimensionCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Fetch `url` and read its pixel dimensions from the image header (no full
/// decode). Returns `None` on any network/decode failure — the caller treats a
/// missing dimension as "render without an explicit aspect ratio".
async fn fetch_dimensions(client: &reqwest::Client, url: &str) -> Option<Dimensions> {
    let bytes = match client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(resp) => resp.bytes().await.ok()?,
        Err(e) => {
            warn!(url, error = %e, "failed to fetch image for dimensions");
            return None;
        }
    };
    match ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
    {
        Ok((width, height)) => {
            debug!(url, width, height, "cached image dimensions");
            Some(Dimensions { width, height })
        }
        Err(e) => {
            warn!(url, error = %e, "failed to read image dimensions");
            None
        }
    }
}

#[derive(Clone)]
pub struct DimensionCacheExtractor(pub Arc<DimensionCache>);

impl FromRequestHead for DimensionCacheExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<DimensionCache>>()
            .cloned()
            .map(DimensionCacheExtractor)
            .ok_or_else(|| cot::Error::internal("DimensionCache not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct DimensionCacheLayer {
    cache: Arc<DimensionCache>,
}

impl DimensionCacheLayer {
    pub const fn new(cache: Arc<DimensionCache>) -> Self {
        Self { cache }
    }
}

impl<S> Layer<S> for DimensionCacheLayer {
    type Service = DimensionCacheService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        DimensionCacheService {
            inner,
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct DimensionCacheService<S> {
    inner: S,
    cache: Arc<DimensionCache>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for DimensionCacheService<S>
where
    S: Service<cot::http::Request<ReqBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: cot::http::Request<ReqBody>) -> Self::Future {
        req.extensions_mut().insert(self.cache.clone());
        self.inner.call(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preset_then_lookup_hits() {
        let cache = DimensionCache::cache_only();
        cache.preset("https://cdn/x@jpeg", 640, 480);
        let dims = cache
            .dimensions("https://cdn/x@jpeg")
            .await
            .expect("preset hit");
        assert_eq!((dims.width, dims.height), (640, 480));
    }

    #[tokio::test]
    async fn miss_returns_none_without_fetching_when_cache_only() {
        let cache = DimensionCache::cache_only();
        assert!(cache.dimensions("https://cdn/missing@jpeg").await.is_none());
    }
}
