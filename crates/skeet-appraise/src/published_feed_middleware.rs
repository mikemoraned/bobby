use std::sync::Arc;
use std::task::{Context, Poll};

use tower::{Layer, Service};

use crate::available_feeds::AvailableFeeds;

/// Injects the configured published feeds (`Arc<AvailableFeeds>`) into request
/// extensions.
///
/// The home page renders whichever published list the viewer selects, joined to
/// live store detail. Read it via [`crate::feed_snapshot::FeedSnapshotSource`].
#[derive(Clone)]
pub struct PublishedFeedLayer {
    feeds: Arc<AvailableFeeds>,
}

impl PublishedFeedLayer {
    pub const fn new(feeds: Arc<AvailableFeeds>) -> Self {
        Self { feeds }
    }
}

impl<S> Layer<S> for PublishedFeedLayer {
    type Service = PublishedFeedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PublishedFeedService {
            inner,
            feeds: self.feeds.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PublishedFeedService<S> {
    inner: S,
    feeds: Arc<AvailableFeeds>,
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
        req.extensions_mut().insert(self.feeds.clone());
        self.inner.call(req)
    }
}
