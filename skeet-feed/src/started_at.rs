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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    #[test]
    fn http_date_format() {
        let dt = chrono::Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        assert_eq!(StartedAt(dt).http_date(), "Sat, 15 Jun 2024 09:30:00 GMT");
    }

    #[test]
    fn http_date_differs_for_different_times() {
        let dt1 = chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let dt2 = chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert_ne!(StartedAt(dt1).http_date(), StartedAt(dt2).http_date());
    }
}
