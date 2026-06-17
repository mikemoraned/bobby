use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use skeet_publish::PublishedImagesSource;
use tower::{Layer, Service};

#[derive(Clone)]
pub struct PublishedImagesSourceExtractor(pub Arc<dyn PublishedImagesSource>);

impl FromRequestHead for PublishedImagesSourceExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<dyn PublishedImagesSource>>()
            .cloned()
            .map(PublishedImagesSourceExtractor)
            .ok_or_else(|| {
                cot::Error::internal("PublishedImagesSource not found in request extensions")
            })
    }
}

#[derive(Clone)]
pub struct PublishedImagesSourceLayer {
    source: Arc<dyn PublishedImagesSource>,
}

impl PublishedImagesSourceLayer {
    pub fn new(source: Arc<dyn PublishedImagesSource>) -> Self {
        Self { source }
    }
}

impl<S> Layer<S> for PublishedImagesSourceLayer {
    type Service = PublishedImagesSourceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PublishedImagesSourceService {
            inner,
            source: self.source.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PublishedImagesSourceService<S> {
    inner: S,
    source: Arc<dyn PublishedImagesSource>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for PublishedImagesSourceService<S>
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
