use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use cot::request::Request;
use cot::response::{IntoResponse, Redirect, Response};
use cot::Error;
use tower::{Layer, Service};
use tracing::warn;

use crate::appraiser_config::resolve_appraiser;

/// Whether a path is reachable without an authenticated appraiser: the health
/// probe, the two login-ceremony endpoints, and genuinely-static chrome (the
/// bundled `/static` assets and a browser's automatic `/favicon.ico`).
///
/// Everything data-bearing stays gated — the home feed, the admin views,
/// appraisal actions, and crucially the skeet *image bytes* (`/skeet/…`), which
/// look like a static asset but serve harvested content. The guard is an
/// allowlist of *public* paths rather than per-route annotations, so a new
/// data route is behind login by default and can't ship unguarded by omission.
fn is_public(path: &str) -> bool {
    const PUBLIC_EXACT: &[&str] = &["/health", "/favicon.ico", "/auth/login", "/auth/callback"];
    PUBLIC_EXACT.contains(&path) || path.starts_with("/static/")
}

/// Root-router middleware that redirects any unauthenticated request for a
/// non-public path to the login flow (preserving the original path as
/// `return_to`). In `--local-admin` mode the appraiser is always present, so
/// every request passes straight through.
#[derive(Clone)]
pub struct RequireAuthLayer;

impl<S> Layer<S> for RequireAuthLayer {
    type Service = RequireAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireAuthService { inner }
    }
}

#[derive(Clone)]
pub struct RequireAuthService<S> {
    inner: S,
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

impl<S> Service<Request> for RequireAuthService<S>
where
    S: Service<Request, Response = Response, Error = Error> + Clone + Send + 'static,
    S::Future: Send,
{
    type Response = Response;
    type Error = Error;
    type Future = BoxFuture<Result<Response, Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        // The inner service may only be ready on the instance we just polled, so
        // swap the ready clone in and call through it (the tower readiness idiom).
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let path = req.uri().path().to_owned();
            if is_public(&path) || resolve_appraiser(req.extensions()).await.is_some() {
                return inner.call(req).await;
            }

            let return_to = req
                .uri()
                .path_and_query()
                .map_or_else(|| path.clone(), |pq| pq.as_str().to_owned());
            warn!(%path, "unauthenticated request; redirecting to login");
            let encoded = urlencoding::encode(&return_to);
            Redirect::new(format!("/auth/login?return_to={encoded}")).into_response()
        })
    }
}
