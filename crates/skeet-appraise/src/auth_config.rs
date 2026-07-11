use std::collections::HashSet;
use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, EndpointNotSet, EndpointSet, RedirectUrl, TokenUrl};
use shared::{BaseUrl, BaseUrlError};
use tower::{Layer, Service};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum OAuthConfigError {
    #[error("invalid auth URL: {0}")]
    AuthUrl(url::ParseError),
    #[error("invalid token URL: {0}")]
    TokenUrl(url::ParseError),
    #[error("invalid GitHub API base URL: {0}")]
    GithubApiBase(BaseUrlError),
}

#[derive(Debug)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub admin_users: HashSet<String>,
    auth_url: Url,
    token_url: Url,
    github_api_base_url: BaseUrl,
}

impl OAuthConfig {
    pub fn new(
        client_id: String,
        client_secret: String,
        admin_users: Vec<String>,
    ) -> Result<Self, OAuthConfigError> {
        Self::with_urls(
            client_id,
            client_secret,
            admin_users,
            "https://github.com/login/oauth/authorize".to_string(),
            "https://github.com/login/oauth/access_token".to_string(),
            "https://api.github.com".to_string(),
        )
    }

    pub fn with_urls(
        client_id: String,
        client_secret: String,
        admin_users: Vec<String>,
        auth_url: String,
        token_url: String,
        github_api_base_url: String,
    ) -> Result<Self, OAuthConfigError> {
        Ok(Self {
            client_id,
            client_secret,
            admin_users: admin_users.into_iter().collect(),
            auth_url: Url::parse(&auth_url).map_err(OAuthConfigError::AuthUrl)?,
            token_url: Url::parse(&token_url).map_err(OAuthConfigError::TokenUrl)?,
            github_api_base_url: BaseUrl::parse(&github_api_base_url)
                .map_err(OAuthConfigError::GithubApiBase)?,
        })
    }

    pub fn build_client(
        &self,
        redirect_url: &Url,
    ) -> BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet> {
        BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_client_secret(ClientSecret::new(self.client_secret.clone()))
            .set_auth_uri(AuthUrl::from_url(self.auth_url.clone()))
            .set_token_uri(TokenUrl::from_url(self.token_url.clone()))
            .set_redirect_uri(RedirectUrl::from_url(redirect_url.clone()))
    }

    /// The GitHub API endpoint for the authenticated user (`{base}/user`).
    pub fn github_user_url(&self) -> Url {
        self.github_api_base_url.join_path(&["user"])
    }

    pub fn is_allowed(&self, username: &str) -> bool {
        self.admin_users.contains(username)
    }
}

pub struct OAuthConfigExtractor(pub Option<Arc<OAuthConfig>>);

impl FromRequestHead for OAuthConfigExtractor {
    async fn from_request_head(head: &RequestHead) -> cot::Result<Self> {
        Ok(Self(head.extensions.get::<Arc<OAuthConfig>>().cloned()))
    }
}

#[derive(Clone)]
pub struct OAuthConfigLayer {
    config: Option<Arc<OAuthConfig>>,
}

impl OAuthConfigLayer {
    pub const fn new(config: Option<Arc<OAuthConfig>>) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for OAuthConfigLayer {
    type Service = OAuthConfigService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OAuthConfigService {
            inner,
            config: self.config.clone(),
        }
    }
}

#[derive(Clone)]
pub struct OAuthConfigService<S> {
    inner: S,
    config: Option<Arc<OAuthConfig>>,
}

impl<S, ReqBody> Service<cot::http::Request<ReqBody>> for OAuthConfigService<S>
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
        if let Some(ref config) = self.config {
            req.extensions_mut().insert(config.clone());
        }
        self.inner.call(req)
    }
}
