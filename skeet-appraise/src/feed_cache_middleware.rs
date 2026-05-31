use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use skeet_publish::FeedCache;
use tower::{Layer, Service};

#[derive(Clone)]
pub struct FeedCacheExtractor(pub Arc<FeedCache>);

impl FromRequestHead for FeedCacheExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<FeedCache>>()
            .cloned()
            .map(FeedCacheExtractor)
            .ok_or_else(|| cot::Error::internal("FeedCache not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct FeedCacheLayer {
    cache: Arc<FeedCache>,
}

impl FeedCacheLayer {
    pub const fn new(cache: Arc<FeedCache>) -> Self {
        Self { cache }
    }
}

impl<S> Layer<S> for FeedCacheLayer {
    type Service = FeedCacheService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FeedCacheService {
            inner,
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct FeedCacheService<S> {
    inner: S,
    cache: Arc<FeedCache>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for FeedCacheService<S>
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
