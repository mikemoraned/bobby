use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};

use skeet_store::SkeetStore;

/// Shared handle to a [`SkeetStore`], extracted from request extensions.
#[derive(Clone)]
pub struct Store(pub Arc<SkeetStore>);

impl FromRequestHead for Store {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<SkeetStore>>()
            .cloned()
            .map(Store)
            .ok_or_else(|| cot::Error::internal("SkeetStore not found in request extensions"))
    }
}

/// Tower [`Layer`] that injects an `Arc<SkeetStore>` into every request's extensions.
#[derive(Clone)]
pub struct StoreLayer {
    store: Arc<SkeetStore>,
}

impl StoreLayer {
    pub fn new(store: SkeetStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub const fn from_shared(store: Arc<SkeetStore>) -> Self {
        Self { store }
    }
}

impl<S> Layer<S> for StoreLayer {
    type Service = StoreService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        StoreService {
            inner,
            store: self.store.clone(),
        }
    }
}

/// Tower [`Service`] that injects an `Arc<SkeetStore>` into every request's extensions.
#[derive(Clone)]
pub struct StoreService<S> {
    inner: S,
    store: Arc<SkeetStore>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for StoreService<S>
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
        req.extensions_mut().insert(self.store.clone());
        self.inner.call(req)
    }
}
