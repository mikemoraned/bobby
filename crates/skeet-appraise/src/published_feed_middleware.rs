use std::sync::Arc;
use std::task::{Context, Poll};

use tower::{Layer, Service};

use crate::available_feeds::PublishedListCatalogReader;

/// Injects the feed-catalog reader (`Arc<PublishedListCatalogReader>`) into request
/// extensions.
///
/// The home page renders whichever published list the viewer selects, joined to
/// live store detail. The available feeds are discovered per render from the
/// publisher's catalog via [`crate::feed_snapshot::FeedSnapshotSource`].
#[derive(Clone)]
pub struct PublishedFeedLayer {
    reader: Arc<PublishedListCatalogReader>,
}

impl PublishedFeedLayer {
    pub const fn new(reader: Arc<PublishedListCatalogReader>) -> Self {
        Self { reader }
    }
}

impl<S> Layer<S> for PublishedFeedLayer {
    type Service = PublishedFeedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PublishedFeedService {
            inner,
            reader: self.reader.clone(),
        }
    }
}

#[derive(Clone)]
pub struct PublishedFeedService<S> {
    inner: S,
    reader: Arc<PublishedListCatalogReader>,
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
        req.extensions_mut().insert(self.reader.clone());
        self.inner.call(req)
    }
}
