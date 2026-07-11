use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use shared::{Did, FeedGeneratorUri, ParseDidError};
use tower::{Layer, Service};
use tracing::warn;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum FeedParamsError {
    #[error("invalid publisher DID: {0}")]
    PublisherDid(ParseDidError),
    #[error("invalid did:web for hostname {hostname:?}: {source}")]
    WebDid {
        hostname: String,
        source: ParseDidError,
    },
    #[error("invalid service endpoint URL: {0}")]
    ServiceEndpoint(url::ParseError),
    #[error("invalid preview image URL: {0}")]
    PreviewImageUrl(url::ParseError),
    #[error("invalid feed bsky URL: {0}")]
    FeedBskyUrl(url::ParseError),
}

#[derive(Debug, Clone)]
pub struct FeedParams {
    publisher_did: Did,
    feed_name: String,
    pub max_entries: usize,
    /// Site-specific Plausible analytics script URL. `None` disables the
    /// tracking script entirely, so only deployments configured with a URL
    /// (i.e. production) load it.
    pub plausible_script_url: Option<String>,
    /// Inline SVG QR code for the site URL, encoded once at construction since
    /// it depends only on `hostname`. `None` if encoding failed (the banner
    /// then renders without it).
    pub site_qr_svg: Option<String>,
    /// The feed generator's own `did:web:{hostname}` identity, validated once at
    /// construction and served in the DID document.
    web_did: Did,
    /// The service's base URL (`https://{hostname}/`), validated once at
    /// construction. Doubles as the canonical site URL ([`Self::site_url`]) and,
    /// in origin form, the DID document's `serviceEndpoint`.
    service_endpoint: Url,
    /// Absolute URL of the social-media preview image (`og:image`).
    preview_image_url: Url,
    /// The `bsky.app` URL where a user can view and subscribe to this feed.
    feed_bsky_url: Url,
}

impl FeedParams {
    pub fn new(
        hostname: String,
        publisher_did: String,
        feed_name: String,
        max_entries: usize,
        plausible_script_url: Option<String>,
    ) -> Result<Self, FeedParamsError> {
        let publisher_did = Did::new(publisher_did).map_err(FeedParamsError::PublisherDid)?;
        let web_did =
            Did::new(format!("did:web:{hostname}")).map_err(|source| FeedParamsError::WebDid {
                hostname: hostname.clone(),
                source,
            })?;
        let service_endpoint =
            Url::parse(&format!("https://{hostname}")).map_err(FeedParamsError::ServiceEndpoint)?;
        let preview_image_url = service_endpoint
            .join(crate::preview::PREVIEW_ROUTE_PATH)
            .map_err(FeedParamsError::PreviewImageUrl)?;
        let feed_bsky_url = Url::parse(&format!(
            "https://bsky.app/profile/{publisher_did}/feed/{feed_name}"
        ))
        .map_err(FeedParamsError::FeedBskyUrl)?;
        let site_qr_svg = crate::qr::qr_svg(service_endpoint.as_str())
            .map_err(|e| warn!(error = %e, "failed to render site QR; banner will omit it"))
            .ok();
        Ok(Self {
            publisher_did,
            feed_name,
            max_entries,
            plausible_script_url,
            site_qr_svg,
            web_did,
            service_endpoint,
            preview_image_url,
            feed_bsky_url,
        })
    }

    /// Override the Plausible analytics script URL (test/config ergonomics).
    #[must_use]
    pub fn with_plausible_script_url(mut self, url: Option<String>) -> Self {
        self.plausible_script_url = url;
        self
    }

    /// The feed generator's `did:web` identity (DID document `id`).
    pub const fn did(&self) -> &Did {
        &self.web_did
    }

    pub fn feed_uri(&self) -> FeedGeneratorUri {
        FeedGeneratorUri::new(&self.publisher_did, &self.feed_name)
    }

    /// The service's base URL, advertised as the feed generator's
    /// `serviceEndpoint` in the DID document.
    pub const fn service_endpoint(&self) -> &Url {
        &self.service_endpoint
    }

    /// The site's own public URL — the canonical page a shared link points at
    /// (`og:url`). This is the service endpoint's base URL.
    pub const fn site_url(&self) -> &Url {
        &self.service_endpoint
    }

    /// Absolute URL of the social-media preview image, for the `og:image` /
    /// `twitter:image` meta tags. Built from the preview route so the two stay
    /// in step.
    pub const fn preview_image_url(&self) -> &Url {
        &self.preview_image_url
    }

    /// The `bsky.app` URL where a user can view and subscribe to this feed.
    pub const fn feed_bsky_url(&self) -> &Url {
        &self.feed_bsky_url
    }
}

#[derive(Debug, Clone)]
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
