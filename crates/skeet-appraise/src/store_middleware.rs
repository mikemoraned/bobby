use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};

use skeet_store::{AppraisalSource, Images, Scores, SkeetStore};

/// The store capabilities the appraisals service needs.
///
/// Aggregates image reads/paging ([`Images`]), score reads ([`Scores`]), and
/// appraisal reads/writes ([`Appraisals`]). Handlers depend on this narrowed
/// surface rather than the whole `SkeetStore`; any type implementing the three
/// ports satisfies it via the blanket impl.
pub trait AppraiseStore: Images + Scores + AppraisalSource {}
impl<T: Images + Scores + AppraisalSource> AppraiseStore for T {}

/// Shared handle to the store, extracted from request extensions.
#[derive(Clone)]
pub struct Store(pub Arc<dyn AppraiseStore>);

impl FromRequestHead for Store {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<dyn AppraiseStore>>()
            .cloned()
            .map(Store)
            .ok_or_else(|| cot::Error::internal("store not found in request extensions"))
    }
}

/// Tower [`Layer`] that injects the store into every request's extensions.
#[derive(Clone)]
pub struct StoreLayer {
    store: Arc<dyn AppraiseStore>,
}

impl StoreLayer {
    pub fn new(store: SkeetStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub fn from_shared(store: Arc<dyn AppraiseStore>) -> Self {
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

/// Tower [`Service`] that injects the store into every request's extensions.
#[derive(Clone)]
pub struct StoreService<S> {
    inner: S,
    store: Arc<dyn AppraiseStore>,
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
