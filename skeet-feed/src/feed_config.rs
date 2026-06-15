use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use tower::{Layer, Service};
use tracing::warn;

#[derive(Debug, Clone)]
pub struct FeedParams {
    pub hostname: String,
    pub publisher_did: String,
    pub feed_name: String,
    pub max_entries: usize,
    /// Site-specific Plausible analytics script URL. `None` disables the
    /// tracking script entirely, so only deployments configured with a URL
    /// (i.e. production) load it.
    pub plausible_script_url: Option<String>,
    /// Inline SVG QR code for the site URL, encoded once at construction since
    /// it depends only on `hostname`. `None` if encoding failed (the banner
    /// then renders without it).
    pub site_qr_svg: Option<String>,
}

/// The site's own public URL — the destination the home-page QR code encodes
/// so a phone scan lands on the feed website.
fn site_url(hostname: &str) -> String {
    format!("https://{hostname}/")
}

impl FeedParams {
    pub fn new(
        hostname: String,
        publisher_did: String,
        feed_name: String,
        max_entries: usize,
        plausible_script_url: Option<String>,
    ) -> Self {
        let site_qr_svg = crate::qr::qr_svg(&site_url(&hostname))
            .map_err(|e| warn!(error = %e, "failed to render site QR; banner will omit it"))
            .ok();
        Self {
            hostname,
            publisher_did,
            feed_name,
            max_entries,
            plausible_script_url,
            site_qr_svg,
        }
    }

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

    /// The `bsky.app` URL where a user can view and subscribe to this feed.
    pub fn feed_bsky_url(&self) -> String {
        format!(
            "https://bsky.app/profile/{}/feed/{}",
            self.publisher_did, self.feed_name
        )
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
