use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};

#[derive(Debug, Clone)]
pub struct FeedParams {
    pub hostname: String,
    pub publisher_did: String,
    pub feed_name: String,
    pub max_entries: usize,
    pub min_score: f32,
    pub max_age_hours: u64,
}

impl FeedParams {
    pub fn did(&self) -> String {
        format!("did:web:{}", self.hostname)
    }

    pub fn feed_uri(&self) -> String {
        format!(
            "at://{}/app.bsky.feed.generator/{}",
            self.publisher_did, self.feed_name
        )
    }

    pub fn service_endpoint(&self) -> String {
        format!("https://{}", self.hostname)
    }
}

#[derive(Clone)]
pub struct FeedConfig(pub Arc<FeedParams>);

impl FromRequestHead for FeedConfig {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<FeedParams>>()
            .cloned()
            .map(FeedConfig)
            .ok_or_else(|| cot::Error::internal("FeedParams not found in request extensions"))
    }
}

#[derive(Clone)]
pub struct FeedConfigLayer {
    config: Arc<FeedParams>,
}

impl FeedConfigLayer {
    pub fn new(params: FeedParams) -> Self {
        Self {
            config: Arc::new(params),
        }
    }
}

impl<S> Layer<S> for FeedConfigLayer {
    type Service = FeedConfigService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FeedConfigService {
            inner,
            config: self.config.clone(),
        }
    }
}

#[derive(Clone)]
pub struct FeedConfigService<S> {
    inner: S,
    config: Arc<FeedParams>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for FeedConfigService<S>
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
        req.extensions_mut().insert(self.config.clone());
        self.inner.call(req)
    }
}
