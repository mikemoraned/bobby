//! Fetching and decoding the montage's thumbnail tiles.
//!
//! Decoded tiles are held in a bounded in-memory cache keyed by URL, so between
//! montages — where the served list shifts by only a few items — most tiles are
//! reused without re-downloading or re-decoding. A tile that fails to download
//! or decode is skipped and never cached, so one bad thumbnail can't sink the
//! montage or poison later attempts.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bluesky::ImageUrl;
use futures::stream::{self, StreamExt};
use image::DynamicImage;
use moka::future::Cache;
use thiserror::Error;
use tracing::warn;

/// Maximum thumbnails downloaded at once.
const FETCH_CONCURRENCY: usize = 6;

#[derive(Debug, Error)]
pub enum TileFetchError {
    #[error("failed to download tile: {0}")]
    Download(String),
    #[error("failed to decode tile image: {0}")]
    Decode(#[from] image::ImageError),
}

/// Source of the raw bytes behind a thumbnail URL. Abstracting the network lets
/// the cache/skip/no-poison behaviour be tested without real downloads.
#[async_trait]
pub trait TileBytesSource: Send + Sync {
    async fn fetch_bytes(&self, url: &ImageUrl) -> Result<Vec<u8>, TileFetchError>;
}

/// A [`TileBytesSource`] backed by HTTP GETs against the Bluesky CDN.
pub struct HttpTileSource {
    client: reqwest::Client,
}

impl HttpTileSource {
    pub const fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl TileBytesSource for HttpTileSource {
    async fn fetch_bytes(&self, url: &ImageUrl) -> Result<Vec<u8>, TileFetchError> {
        let response = self
            .client
            .get(url.to_string())
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| TileFetchError::Download(e.to_string()))?;
        let bytes = response
            .bytes()
            .await
            .map_err(|e| TileFetchError::Download(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

/// Fetches montage tiles through a bounded cache of decoded images.
pub struct TileFetcher<S: TileBytesSource> {
    source: S,
    cache: Cache<ImageUrl, Arc<DynamicImage>>,
}

impl<S: TileBytesSource> TileFetcher<S> {
    /// A fetcher caching up to `max_cached_tiles` decoded images.
    pub fn new(source: S, max_cached_tiles: u64) -> Self {
        Self {
            source,
            cache: Cache::new(max_cached_tiles),
        }
    }

    /// Fetch (or reuse cached) decoded tiles for `urls`, keyed by URL.
    ///
    /// A tile that fails to download or decode is absent from the map (and not
    /// cached), so the caller composes from whatever succeeded — looking each
    /// tile up by URL to realign it with the selection order.
    pub async fn fetch(&self, urls: &[ImageUrl]) -> HashMap<ImageUrl, Arc<DynamicImage>> {
        stream::iter(urls.iter().cloned())
            .map(|url| self.fetch_one(url))
            .buffer_unordered(FETCH_CONCURRENCY)
            .filter_map(|entry| async move { entry })
            .collect()
            .await
    }

    /// Fetch one tile through the cache, yielding `None` (after logging) on
    /// failure.
    async fn fetch_one(&self, url: ImageUrl) -> Option<(ImageUrl, Arc<DynamicImage>)> {
        let load = async {
            let bytes = self.source.fetch_bytes(&url).await?;
            let image = image::load_from_memory(&bytes)?;
            Ok::<_, TileFetchError>(Arc::new(image))
        };
        // `try_get_with` coalesces concurrent requests for the same URL
        // and caches only successes.
        match self.cache.try_get_with(url.clone(), load).await {
            Ok(image) => Some((url, image)),
            Err(error) => {
                warn!(%url, %error, "skipping montage tile");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use image::{Rgb, RgbImage};

    use super::*;

    /// A scripted bytes source: each URL has a queue of responses consumed one
    /// per call, and per-URL call counts so caching can be asserted.
    #[derive(Default)]
    struct ScriptedSource {
        responses: Mutex<HashMap<String, VecDeque<Result<Vec<u8>, ()>>>>,
        calls: Mutex<HashMap<String, usize>>,
    }

    impl ScriptedSource {
        fn push(&self, url: &ImageUrl, response: Result<Vec<u8>, ()>) {
            self.responses
                .lock()
                .expect("lock")
                .entry(url.to_string())
                .or_default()
                .push_back(response);
        }

        fn calls_for(&self, url: &ImageUrl) -> usize {
            self.calls
                .lock()
                .expect("lock")
                .get(&url.to_string())
                .copied()
                .unwrap_or(0)
        }
    }

    #[async_trait]
    impl TileBytesSource for ScriptedSource {
        async fn fetch_bytes(&self, url: &ImageUrl) -> Result<Vec<u8>, TileFetchError> {
            *self
                .calls
                .lock()
                .expect("lock")
                .entry(url.to_string())
                .or_default() += 1;
            let response = self
                .responses
                .lock()
                .expect("lock")
                .get_mut(&url.to_string())
                .and_then(VecDeque::pop_front);
            match response {
                Some(Ok(bytes)) => Ok(bytes),
                _ => Err(TileFetchError::Download("scripted failure".to_string())),
            }
        }
    }

    fn url(seed: &str) -> ImageUrl {
        ImageUrl::new(format!("https://cdn.bsky.app/img/{seed}.jpg")).expect("valid url")
    }

    /// A tiny valid PNG (a solid 4×4 image).
    fn png_bytes(colour: [u8; 3]) -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb(colour)));
        let mut buffer = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode png");
        buffer.into_inner()
    }

    fn fetcher(source: ScriptedSource) -> TileFetcher<ScriptedSource> {
        TileFetcher::new(source, 32)
    }

    fn pixel(tile: &Arc<DynamicImage>) -> Rgb<u8> {
        *tile.to_rgb8().get_pixel(0, 0)
    }

    #[tokio::test]
    async fn fetches_and_decodes_each_requested_tile() {
        let source = ScriptedSource::default();
        source.push(&url("a"), Ok(png_bytes([200, 0, 0])));
        source.push(&url("b"), Ok(png_bytes([0, 200, 0])));
        let fetcher = fetcher(source);

        let tiles = fetcher.fetch(&[url("a"), url("b")]).await;
        assert_eq!(tiles.len(), 2);
        assert_eq!(
            pixel(&tiles[&url("a")]),
            Rgb([200, 0, 0]),
            "url a decoded to red"
        );
        assert_eq!(
            pixel(&tiles[&url("b")]),
            Rgb([0, 200, 0]),
            "url b decoded to green"
        );
    }

    #[tokio::test]
    async fn cached_tiles_are_not_refetched() {
        let source = ScriptedSource::default();
        source.push(&url("a"), Ok(png_bytes([1, 2, 3])));
        let fetcher = fetcher(source);

        fetcher.fetch(&[url("a")]).await;
        let second = fetcher.fetch(&[url("a")]).await;
        assert!(
            second.contains_key(&url("a")),
            "the tile is still returned on the second call"
        );
        // The cache proof: the source was hit once across both calls.
        assert_eq!(
            fetcher.source.calls_for(&url("a")),
            1,
            "a cached URL must not be downloaded again"
        );
    }

    #[tokio::test]
    async fn a_failed_tile_is_dropped_and_the_others_are_still_returned() {
        let source = ScriptedSource::default();
        source.push(&url("a"), Ok(png_bytes([1, 1, 1])));
        source.push(&url("b"), Err(())); // download failure
        let fetcher = fetcher(source);

        let tiles = fetcher.fetch(&[url("a"), url("b")]).await;
        assert_eq!(tiles.len(), 1);
        assert_eq!(
            pixel(&tiles[&url("a")]),
            Rgb([1, 1, 1]),
            "the good tile is returned"
        );
        assert!(!tiles.contains_key(&url("b")), "the failed tile is absent");
    }

    #[tokio::test]
    async fn failures_are_not_cached_so_a_later_attempt_can_succeed() {
        let source = ScriptedSource::default();
        // First attempt fails, second succeeds.
        source.push(&url("b"), Err(()));
        source.push(&url("b"), Ok(png_bytes([9, 9, 9])));
        let fetcher = fetcher(source);

        assert!(
            fetcher.fetch(&[url("b")]).await.is_empty(),
            "first attempt fails"
        );
        assert!(
            fetcher.fetch(&[url("b")]).await.contains_key(&url("b")),
            "retry succeeds because the failure wasn't cached"
        );
        assert_eq!(
            fetcher.source.calls_for(&url("b")),
            2,
            "the failure must not be cached — the URL is fetched again"
        );
    }

    #[tokio::test]
    async fn undecodable_bytes_are_skipped_and_not_cached() {
        let source = ScriptedSource::default();
        source.push(&url("b"), Ok(b"not a png".to_vec()));
        source.push(&url("b"), Ok(png_bytes([5, 5, 5])));
        let fetcher = fetcher(source);

        assert!(
            fetcher.fetch(&[url("b")]).await.is_empty(),
            "undecodable is skipped"
        );
        assert!(
            fetcher.fetch(&[url("b")]).await.contains_key(&url("b")),
            "a decode failure isn't cached, so a later valid image works"
        );
    }
}
