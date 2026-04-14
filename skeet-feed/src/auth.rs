use cot::request::extractors::UrlQuery;
use cot::response::{IntoResponse, Redirect, Response};
use cot::session::Session;
use cot::{Body, StatusCode};
use oauth2::{AuthorizationCode, CsrfToken, Scope, TokenResponse};
use serde::Deserialize;
use shared::Appraiser;
use tracing::{info, warn};

use crate::auth_config::OAuthConfigExtractor;

#[derive(Deserialize)]
pub struct LoginQuery {
    pub return_to: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Deserialize)]
struct GitHubUser {
    login: String,
}

fn session_err(e: impl std::fmt::Display) -> cot::Error {
    cot::Error::internal(format!("session error: {e}"))
}

pub async fn auth_login(
    session: Session,
    OAuthConfigExtractor(config): OAuthConfigExtractor,
    UrlQuery(query): UrlQuery<LoginQuery>,
) -> cot::Result<Response> {
    let config = config
        .ok_or_else(|| cot::Error::internal("OAuth not configured — use --local-admin for local dev"))?;

    let (auth_url, csrf_state) = config
        .client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("read:user".to_string()))
        .url();

    session
        .insert("csrf_state", csrf_state.secret().to_string())
        .await
        .map_err(session_err)?;

    if let Some(return_to) = query.return_to {
        session
            .insert("return_to", return_to)
            .await
            .map_err(session_err)?;
    }

    info!("redirecting to GitHub OAuth");
    Redirect::new(auth_url.to_string()).into_response()
}

pub async fn auth_callback(
    session: Session,
    OAuthConfigExtractor(config): OAuthConfigExtractor,
    UrlQuery(query): UrlQuery<CallbackQuery>,
) -> cot::Result<Response> {
    let config = config
        .ok_or_else(|| cot::Error::internal("OAuth not configured"))?;

    // Verify CSRF state
    let stored_state: Option<String> = session.get("csrf_state").await.map_err(session_err)?;
    if stored_state.as_deref() != Some(&query.state) {
        warn!("CSRF state mismatch");
        let mut response = Response::new(Body::fixed("CSRF state mismatch — please try logging in again"));
        *response.status_mut() = StatusCode::FORBIDDEN;
        return Ok(response);
    }
    session
        .remove::<String>("csrf_state")
        .await
        .map_err(session_err)?;

    // Exchange code for access token
    let http_client = reqwest::Client::new();
    let token_result = config
        .client
        .exchange_code(AuthorizationCode::new(query.code))
        .request_async(&http_client)
        .await
        .map_err(|e| cot::Error::internal(format!("token exchange failed: {e}")))?;

    let access_token = token_result.access_token().secret();

    // Fetch GitHub username
    let user_response = http_client
        .get(format!("{}/user", config.github_api_base_url))
        .header("authorization", format!("Bearer {access_token}"))
        .header("user-agent", "bobby-feed")
        .send()
        .await
        .map_err(|e| cot::Error::internal(format!("GitHub API error: {e}")))?;

    if !user_response.status().is_success() {
        let mut response = Response::new(Body::fixed("Failed to fetch GitHub user info"));
        *response.status_mut() = StatusCode::FORBIDDEN;
        return Ok(response);
    }

    let github_user: GitHubUser = user_response
        .json()
        .await
        .map_err(|e| cot::Error::internal(format!("GitHub user parse error: {e}")))?;

    // Check allowlist
    if !config.is_allowed(&github_user.login) {
        warn!(username = %github_user.login, "user not in admin allowlist");
        let mut response = Response::new(Body::fixed(format!(
            "Access denied: {} is not in the admin allowlist",
            github_user.login
        )));
        *response.status_mut() = StatusCode::FORBIDDEN;
        return Ok(response);
    }

    info!(username = %github_user.login, "admin login successful");

    // Store role and appraiser in session
    let appraiser = Appraiser::GitHub {
        username: github_user.login,
    };
    session
        .insert("role", "admin".to_string())
        .await
        .map_err(session_err)?;
    session
        .insert("appraiser", appraiser.to_string())
        .await
        .map_err(session_err)?;

    // Redirect to return_to or /admin
    let return_to: Option<String> = session.get("return_to").await.map_err(session_err)?;
    session
        .remove::<String>("return_to")
        .await
        .map_err(session_err)?;

    let destination = return_to.unwrap_or_else(|| "/admin".to_string());
    Redirect::new(destination).into_response()
}

pub async fn auth_logout(session: Session) -> cot::Result<Response> {
    session.flush().await.map_err(session_err)?;
    info!("admin logged out");
    Redirect::new("/").into_response()
}
