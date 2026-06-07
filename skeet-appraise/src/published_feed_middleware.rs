use std::sync::Arc;
use std::task::{Context, Poll};

use skeet_publish::RedisFeedSource;
use tower::{Layer, Service};

/// Injects the published-feed reader (`Arc<RedisFeedSource>`) into request extensions.
///
/// The home page renders exactly what the Bluesky feed publishes, joined to live
/// store detail. Read it via [`crate::feed_snapshot::FeedSnapshotSource`].
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
