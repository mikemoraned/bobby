//! Integration tests that exercise the Redis session store path end-to-end.
//!
//! A testcontainers Redis is started, a full `AppraiseProject` is wired to use
//! it with OAuth authentication, and the login flow is exercised so that
//! sessions are actually created/loaded in Redis.
//!
//! Requires Docker. Gated behind `integ` (which implies `test`).

#![cfg(feature = "integ")]

mod common;

use std::sync::Arc;

use chrono::Utc;
use common::{extract_query_param, extract_session_cookie, get_with_cookie, mount_github_mocks};
use cot::test::Client;
use rcgen::{CertificateParams, KeyPair};
use skeet_appraise::auth_config::OAuthConfig;
use skeet_appraise::available_feeds::PublishedListCatalogReader;
use skeet_appraise::project::AppraiseProject;
use skeet_appraise::{
    AppraiserLayer, ModelsLayer, OAuthConfigLayer, PublishedFeedLayer, StartedAtLayer, StoreLayer,
};
use skeet_store::test_utils::{make_record, open_temp_store};
use skeet_store::{ModelVersion, Score, SkeetStore};
use test_support::test_models;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, CopyTargetOptions, GenericImage, ImageExt};
use wiremock::MockServer;

const REDIS_TLS_PORT: u16 = 6380;

/// The login-flow tests never hit the home page, so the published-feed reader is
/// pointed at an unreachable url.
const DUMMY_PUBLISH_URL: &str = "redis://127.0.0.1:1";

async fn oauth_client_with_redis(
    mock_server: &MockServer,
    redis_url: &str,
    store: Arc<SkeetStore>,
) -> Client {
    let oauth_config = OAuthConfig::with_urls(
        "test-client-id".to_string(),
        "test-client-secret".to_string(),
        vec!["testuser".to_string()],
        format!("{}/authorize", mock_server.uri()),
        format!("{}/token", mock_server.uri()),
        mock_server.uri().to_string(),
    );
    let project = AppraiseProject {
        published_feed_layer: PublishedFeedLayer::new(Arc::new(PublishedListCatalogReader::new(
            DUMMY_PUBLISH_URL,
        ))),
        store_layer: StoreLayer::from_shared(store),
        models_layer: ModelsLayer::from_shared(test_models()),
        appraiser_layer: AppraiserLayer::new(None),
        oauth_config_layer: OAuthConfigLayer::new(Some(Arc::new(oauth_config))),
        started_at_layer: StartedAtLayer::new(Utc::now()),
        session_secret: Some("test-secret-at-least-32-bytes-long-for-hmac".to_string()),
        use_redis: true,
        redis_url: Some(redis_url.to_string()),
    };
    Client::new(project).await
}

/// Perform a full login flow: /auth/login -> capture state -> /auth/callback.
/// Returns the session cookie.
async fn do_login(client: &mut Client) -> String {
    let response = client
        .request(get_with_cookie("/auth/login", None))
        .await
        .expect("GET /auth/login");
    if response.status().as_u16() != 303 {
        let status = response.status().as_u16();
        let body_bytes = response.into_body().into_bytes().await.expect("read body");
        let body = String::from_utf8(body_bytes.to_vec()).expect("valid utf8");
        panic!("login should redirect (303), got {status}: {body}");
    }
    let location = response
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header")
        .to_string();
    let session_cookie = extract_session_cookie(&response).expect("session cookie set");
    let state = extract_query_param(&location, "state").expect("state param in redirect URL");

    let callback_uri = format!("/auth/callback?code=test-code&state={state}");
    let response = client
        .request(get_with_cookie(&callback_uri, Some(&session_cookie)))
        .await
        .expect("GET /auth/callback");
    assert_eq!(
        response.status().as_u16(),
        303,
        "callback should redirect after successful login"
    );

    extract_session_cookie(&response).unwrap_or(session_cookie)
}

async fn seed_store(store: &SkeetStore, suffix: &str) {
    let record = make_record(suffix, 10, 0, 0);
    let image_id = record.image_id.clone();
    store.add(&record).await.expect("add record");
    store
        .upsert_score(
            &image_id,
            &Score::new(0.85).expect("valid score"),
            &ModelVersion::from("test"),
        )
        .await
        .expect("upsert score");
}

/// Start a Redis container with TLS enabled using self-signed certs.
async fn start_redis_with_tls() -> (ContainerAsync<GenericImage>, String) {
    let key_pair = KeyPair::generate().expect("generate key pair");
    let params = CertificateParams::new(vec!["localhost".to_string()]).expect("cert params");
    let cert = params.self_signed(&key_pair).expect("self-sign cert");

    let cert_pem = cert.pem().into_bytes();
    let key_pem = key_pair.serialize_pem().into_bytes();

    let container = GenericImage::new("redis", "7")
        .with_exposed_port(REDIS_TLS_PORT.tcp())
        .with_copy_to(
            CopyTargetOptions::new("/tls/redis.crt").with_mode(0o644),
            cert_pem.clone(),
        )
        .with_copy_to(
            CopyTargetOptions::new("/tls/redis.key").with_mode(0o644),
            key_pem,
        )
        .with_copy_to(
            CopyTargetOptions::new("/tls/ca.crt").with_mode(0o644),
            cert_pem,
        )
        .with_cmd([
            "redis-server",
            "--tls-port",
            "6380",
            "--port",
            "0",
            "--tls-cert-file",
            "/tls/redis.crt",
            "--tls-key-file",
            "/tls/redis.key",
            "--tls-ca-cert-file",
            "/tls/ca.crt",
            "--tls-auth-clients",
            "no",
        ])
        .with_ready_conditions(vec![WaitFor::message_on_stdout(
            "Ready to accept connections",
        )])
        .start()
        .await
        .expect("start Redis TLS container");

    let host = container.get_host().await.expect("get host");
    let port = container
        .get_host_port_ipv4(REDIS_TLS_PORT.tcp())
        .await
        .expect("get mapped TLS port");
    let url = format!("rediss://{host}:{port}/#insecure");

    (container, url)
}

// ─── Plain TCP ────────────────────────────────────────────────

#[tokio::test]
async fn login_and_admin_with_redis_session_store_docker() {
    let container = testcontainers_modules::redis::Redis::default()
        .start()
        .await
        .expect("start Redis container");
    let host = container.get_host().await.expect("get host");
    let port = container
        .get_host_port_ipv4(6379)
        .await
        .expect("get mapped port");
    let redis_url = format!("redis://{host}:{port}");

    let mock_server = MockServer::start().await;
    mount_github_mocks(&mock_server, "testuser").await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    seed_store(&store, "redis-tcp").await;
    let store = Arc::new(store);

    let mut client = oauth_client_with_redis(&mock_server, &redis_url, store).await;

    // Without login, admin redirects to login
    let response = client
        .request(get_with_cookie("/admin", None))
        .await
        .expect("GET /admin");
    assert_eq!(response.status().as_u16(), 303);

    // Login — session is created in Redis
    let cookie = do_login(&mut client).await;

    // Authenticated admin request — session is loaded from Redis
    let response = client
        .request(get_with_cookie("/admin", Some(&cookie)))
        .await
        .expect("GET /admin after login");
    assert_eq!(response.status().as_u16(), 200);
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("valid utf8");
    assert!(
        body.contains("redis-tcp"),
        "admin page should show seeded record"
    );
}

// ─── TLS ──────────────────────────────────────────────────────

#[tokio::test]
async fn login_and_admin_with_redis_over_tls_docker() {
    // Reproduces the original deploy bug: cot's RedisStore with a `rediss://`
    // URL failed at startup with "can't connect with TLS, the feature is not
    // enabled". This test starts Redis with TLS (self-signed certs), does a
    // full OAuth login, and accesses admin — exercising session create/load
    // over a real TLS connection to Redis.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let (_container, redis_url) = start_redis_with_tls().await;

    let mock_server = MockServer::start().await;
    mount_github_mocks(&mock_server, "testuser").await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let store = open_temp_store(&dir).await;
    seed_store(&store, "redis-tls").await;
    let store = Arc::new(store);

    let mut client = oauth_client_with_redis(&mock_server, &redis_url, store).await;

    // Login — session is created in Redis over TLS
    let cookie = do_login(&mut client).await;

    // Authenticated admin request — session is loaded from Redis over TLS
    let response = client
        .request(get_with_cookie("/admin", Some(&cookie)))
        .await
        .expect("GET /admin after login");
    assert_eq!(response.status().as_u16(), 200);
    let body_bytes = response.into_body().into_bytes().await.expect("read body");
    let body = String::from_utf8(body_bytes.to_vec()).expect("valid utf8");
    assert!(
        body.contains("redis-tls"),
        "admin page should show seeded record via TLS Redis"
    );
}
