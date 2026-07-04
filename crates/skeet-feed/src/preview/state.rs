//! Shared, request-scoped state for the preview route: the montage cache and the
//! tile fetcher, threaded into handlers through request extensions.

use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};

use super::montage::MontageCache;
use super::selection::MONTAGE_TILE_COUNT;
use super::tiles::{HttpTileSource, TileFetcher};

/// How many decoded tiles the fetch cache holds: a few montages' worth, so the
/// tiles shared between successive served lists stay warm.
const TILE_CACHE_CAPACITY: u64 = MONTAGE_TILE_COUNT as u64 * 3;

/// The preview route's long-lived state, built once and shared across requests.
pub struct PreviewState {
    pub cache: MontageCache,
    pub fetcher: TileFetcher<HttpTileSource>,
    pub size: (u32, u32),
}

impl PreviewState {
    pub fn new(client: reqwest::Client, size: (u32, u32)) -> Self {
        Self {
            cache: MontageCache::new(),
            fetcher: TileFetcher::new(HttpTileSource::new(client), TILE_CACHE_CAPACITY),
            size,
        }
    }
}

#[derive(Clone)]
pub struct PreviewStateExtractor(pub Arc<PreviewState>);

impl FromRequestHead for PreviewStateExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<PreviewState>>()
            .cloned()
            .map(PreviewStateExtractor)
            .ok_or_else(|| cot::Error::internal("PreviewState not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct PreviewStateLayer {
    state: Arc<PreviewState>,
}

impl PreviewStateLayer {
    pub const fn new(state: Arc<PreviewState>) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for PreviewStateLayer {
    type Service = PreviewStateService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PreviewStateService {
            inner,
            state: self.state.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PreviewStateService<S> {
    inner: S,
    state: Arc<PreviewState>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for PreviewStateService<S>
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
        req.extensions_mut().insert(self.state.clone());
        self.inner.call(req)
    }
}
