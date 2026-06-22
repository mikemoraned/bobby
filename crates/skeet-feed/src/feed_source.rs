use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use skeet_publish::FeedSource;
use tower::{Layer, Service};

#[derive(Clone)]
pub struct FeedSourceExtractor(pub Arc<dyn FeedSource>);

impl FromRequestHead for FeedSourceExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<dyn FeedSource>>()
            .cloned()
            .map(FeedSourceExtractor)
            .ok_or_else(|| cot::Error::internal("FeedSource not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct FeedSourceLayer {
    source: Arc<dyn FeedSource>,
}

impl FeedSourceLayer {
    pub fn new(source: Arc<dyn FeedSource>) -> Self {
        Self { source }
    }
}

impl<S> Layer<S> for FeedSourceLayer {
    type Service = FeedSourceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FeedSourceService {
            inner,
            source: self.source.clone(),
        }
    }
}

#[derive(Clone)]
pub struct FeedSourceService<S> {
    inner: S,
    source: Arc<dyn FeedSource>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for FeedSourceService<S>
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
        req.extensions_mut().insert(self.source.clone());
        self.inner.call(req)
    }
}
