use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use shared::Appraiser;
use tower::{Layer, Service};

/// Extracts the current appraiser (if any) from request extensions.
///
/// Checks two sources in order:
/// 1. Static `Arc<Appraiser>` in extensions (set by `AppraiserLayer` in `--local-admin` mode)
/// 2. Session `appraiser` key (set by GitHub OAuth callback)
#[derive(Clone)]
pub struct AppraiserExtractor(pub Option<Arc<Appraiser>>);

impl FromRequestHead for AppraiserExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        // Local-admin mode: static appraiser in extensions
        if let Some(appraiser) = head.extensions.get::<Arc<Appraiser>>() {
            return Ok(Self(Some(appraiser.clone())));
        }

        // OAuth mode: appraiser stored in session
        if let Some(session) = head.extensions.get::<cot::session::Session>()
            && let Ok(Some(appraiser_str)) = session.get::<String>("appraiser").await
            && let Ok(appraiser) = appraiser_str.parse::<Appraiser>()
        {
            return Ok(Self(Some(Arc::new(appraiser))));
        }

        Ok(Self(None))
    }
}

/// Tower [`Layer`] that injects an optional `Arc<Appraiser>` into request extensions.
#[derive(Clone)]
pub struct AppraiserLayer {
    appraiser: Option<Arc<Appraiser>>,
}

impl AppraiserLayer {
    pub const fn new(appraiser: Option<Arc<Appraiser>>) -> Self {
        Self { appraiser }
    }
}

impl<S> Layer<S> for AppraiserLayer {
    type Service = AppraiserService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AppraiserService {
            inner,
            appraiser: self.appraiser.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AppraiserService<S> {
    inner: S,
    appraiser: Option<Arc<Appraiser>>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for AppraiserService<S>
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
        if let Some(ref appraiser) = self.appraiser {
            req.extensions_mut().insert(appraiser.clone());
        }
        self.inner.call(req)
    }
}
