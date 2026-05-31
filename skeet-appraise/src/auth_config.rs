use std::collections::HashSet;
use std::sync::Arc;
use std::task::{Context, Poll};

use cot::http::request::Parts as RequestHead;
use cot::request::extractors::FromRequestHead;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, EndpointNotSet, EndpointSet, RedirectUrl, TokenUrl};
use tower::{Layer, Service};

pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub admin_users: HashSet<String>,
    pub auth_url: String,
    pub token_url: String,
    pub github_api_base_url: String,
}

impl OAuthConfig {
    pub fn new(
        client_id: String,
        client_secret: String,
        admin_users: Vec<String>,
    ) -> Self {
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
    ) -> Self {
        Self {
            client_id,
            client_secret,
            admin_users: admin_users.into_iter().collect(),
            auth_url,
            token_url,
            github_api_base_url,
        }
    }

    pub fn build_client(&self, redirect_url: &str) -> BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet> {
        BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_client_secret(ClientSecret::new(self.client_secret.clone()))
            .set_auth_uri(AuthUrl::new(self.auth_url.clone()).expect("valid auth URL"))
            .set_token_uri(TokenUrl::new(self.token_url.clone()).expect("valid token URL"))
            .set_redirect_uri(RedirectUrl::new(redirect_url.to_string()).expect("valid redirect URL"))
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
