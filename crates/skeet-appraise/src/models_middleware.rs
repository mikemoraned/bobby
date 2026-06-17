use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};

use shared::RefineModels;

/// Shared handle to the [`RefineModels`] registry, extracted from request extensions.
///
/// The display badges resolve each score → band via the producing model's
/// threshold, so the admin and home pages need the same registry the feed uses.
#[derive(Clone)]
pub struct Models(pub Arc<RefineModels>);

impl FromRequestHead for Models {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        head.extensions
            .get::<Arc<RefineModels>>()
            .cloned()
            .map(Models)
            .ok_or_else(|| cot::Error::internal("RefineModels not found in request extensions"))
    }
}

/// Tower [`Layer`] that injects an `Arc<RefineModels>` into every request's extensions.
#[derive(Clone)]
pub struct ModelsLayer {
    models: Arc<RefineModels>,
}

impl ModelsLayer {
    pub const fn from_shared(models: Arc<RefineModels>) -> Self {
        Self { models }
    }
}

impl<S> Layer<S> for ModelsLayer {
    type Service = ModelsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ModelsService {
            inner,
            models: self.models.clone(),
        }
    }
}

/// Tower [`Service`] that injects an `Arc<RefineModels>` into every request's extensions.
#[derive(Clone)]
pub struct ModelsService<S> {
    inner: S,
    models: Arc<RefineModels>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for ModelsService<S>
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
        req.extensions_mut().insert(self.models.clone());
        self.inner.call(req)
    }
}
