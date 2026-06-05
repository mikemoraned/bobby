use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use skeet_publish::RedisFeedSource;
use tower::{Layer, Service};

/// Injects the published-feed reader so the home page renders exactly what the
/// Bluesky feed publishes (the `recency-48h` list), joined to live store detail.
#[derive(Clone)]
pub struct PublishedFeedExtractor(pub Arc<RedisFeedSource>);

impl FromRequestHead for PublishedFeedExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<RedisFeedSource>>()
            .cloned()
            .map(PublishedFeedExtractor)
            .ok_or_else(|| cot::Error::internal("RedisFeedSource not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct PublishedFeedLayer {
    feed: Arc<RedisFeedSource>,
}

impl PublishedFeedLayer {
    pub const fn new(feed: Arc<RedisFeedSource>) -> Self {
        Self { feed }
    }
}

impl<S> Layer<S> for PublishedFeedLayer {
    type Service = PublishedFeedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PublishedFeedService {
            inner,
            feed: self.feed.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PublishedFeedService<S> {
    inner: S,
    feed: Arc<RedisFeedSource>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for PublishedFeedService<S>
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
        req.extensions_mut().insert(self.feed.clone());
        self.inner.call(req)
    }
}
