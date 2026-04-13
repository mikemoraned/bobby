use std::sync::Arc;
use std::task::{Context, Poll};

use chrono::{DateTime, Utc};
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};

/// Wall-clock time when the server started, used for HTTP cache headers.
#[derive(Clone, Debug)]
pub struct StartedAt(pub DateTime<Utc>);

impl StartedAt {
    /// Format as an HTTP-date for use in `Last-Modified` / `Date` headers.
    pub fn http_date(&self) -> String {
        self.0.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
    }
}

/// Extracts `StartedAt` from request extensions.
pub struct StartedAtExtractor(pub StartedAt);

impl FromRequestHead for StartedAtExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<StartedAt>>()
            .cloned()
            .map(|a| Self(StartedAt(a.0)))
            .ok_or_else(|| cot::Error::internal("StartedAt not found in request extensions"))
    }
}

/// Tower [`Layer`] that injects `Arc<StartedAt>` into request extensions.
#[derive(Clone)]
pub struct StartedAtLayer {
    started_at: Arc<StartedAt>,
}

impl StartedAtLayer {
    pub fn new(started_at: DateTime<Utc>) -> Self {
        Self {
            started_at: Arc::new(StartedAt(started_at)),
        }
    }
}

impl<S> Layer<S> for StartedAtLayer {
    type Service = StartedAtService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        StartedAtService {
            inner,
            started_at: self.started_at.clone(),
        }
    }
}

#[derive(Clone)]
pub struct StartedAtService<S> {
    inner: S,
    started_at: Arc<StartedAt>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for StartedAtService<S>
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
        req.extensions_mut().insert(self.started_at.clone());
        self.inner.call(req)
    }
}
