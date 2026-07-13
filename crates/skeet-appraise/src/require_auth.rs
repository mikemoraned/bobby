use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use cot::request::Request;
use cot::response::{IntoResponse, Redirect, Response};
use cot::Error;
use tower::{Layer, Service};
use tracing::warn;

use crate::appraiser_config::resolve_appraiser;

/// Paths reachable without an authenticated appraiser. Everything else — every
/// page, image, and admin action — requires a session.
///
/// The guard is expressed as an allowlist of *public* paths rather than by
/// annotating each protected route, so a newly-added route is behind login by
/// default and cannot ship unguarded by omission.
const PUBLIC_PATHS: &[&str] = &["/auth/login", "/auth/callback"];

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
            if PUBLIC_PATHS.contains(&path.as_str())
                || resolve_appraiser(req.extensions()).await.is_some()
            {
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
