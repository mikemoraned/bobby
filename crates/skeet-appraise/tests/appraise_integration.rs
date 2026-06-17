//! Integration tests that hit a running skeet-appraise server via HTTP.
//!
//! When `TEST_BASE_URL` is set (e.g. `end_to_end_test_appraise`), tests hit that
//! URL. Otherwise, each test spawns its own `skeet-appraise` subprocess on a
//! free port against a fresh tempdir store; the `TestServer` Drop guard kills
//! the child when the test ends. nextest runs each test in its own process, so a
//! single shared server isn't viable here.
//!
//! These assert only the *unauthenticated* surface so they hold identically
//! against a locally-spawned server (no `--local-admin`, in-memory sessions) and
//! the live OAuth-protected staging deployment: `/` renders, the static assets
//! are served, and `/admin` redirects to login.
//!
//! The locally-spawned server is storeless-for-the-feed but still needs a redis
//! publish url to start, so the local path runs a testcontainers redis and the
//! tests are `_docker`-suffixed; the `TEST_BASE_URL` staging path skips it.
//!
//! Gated behind the `integ` feature so `just test` doesn't compile them.

#![cfg(feature = "integ")]

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::redis::{REDIS_PORT, Redis};

struct TestServer {
    child: Option<Child>,
    _store: Option<tempfile::TempDir>,
    _redis: Option<ContainerAsync<Redis>>,
    url: String,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn spawn_server() -> TestServer {
    if let Ok(url) = std::env::var("TEST_BASE_URL") {
        reqwest::get(&url)
            .await
            .unwrap_or_else(|e| panic!("server not reachable at {url}: {e}"));
        return TestServer {
            child: None,
            _store: None,
            _redis: None,
            url,
        };
    }

    let container = Redis::default().start().await.expect("start redis container");
    let host = container.get_host().await.expect("redis host");
    let redis_port = container
        .get_host_port_ipv4(REDIS_PORT)
        .await
        .expect("redis port");
    let redis_url = format!("redis://{host}:{redis_port}");
    wait_redis_ready(&redis_url).await;

    let port = pick_free_port();
    let store = tempfile::tempdir().expect("create temp store");
    let bind = format!("127.0.0.1:{port}");
    let bin = env!("CARGO_BIN_EXE_skeet-appraise");
    // The binary loads the model registry at startup; the default `config/refine.toml`
    // is relative to CWD (the crate dir under nextest), so point at the repo's copy.
    let model_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/refine.toml");
    let child = Command::new(bin)
        .args([
            "--store-path",
            store.path().to_str().expect("utf-8 store path"),
            "--bind",
            &bind,
            "--redis-publish-url",
            &redis_url,
            "--model-path",
            model_path,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn skeet-appraise");

    let url = format!("http://127.0.0.1:{port}");
    let server = TestServer {
        child: Some(child),
        _store: Some(store),
        _redis: Some(container),
        url: url.clone(),
    };
    for _ in 0..30 {
        if reqwest::get(&url).await.is_ok() {
            return server;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("local skeet-appraise server failed to become reachable at {url} within 15s");
}

/// Wait until the testcontainers redis actually answers.
async fn wait_redis_ready(url: &str) {
    for _ in 0..100 {
        if skeet_publish::connect(url).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("redis container not reachable at {url} within 10s");
}

fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener
        .local_addr()
        .expect("local addr")
        .port()
}

#[tokio::test]
async fn home_page_renders_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    let resp = client
        .get(base)
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200, "home page should render");

    let body = resp.text().await.expect("body text");
    assert!(
        body.contains("<html"),
        "home page should return an HTML document"
    );
}

#[tokio::test]
async fn htmx_static_asset_is_served_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/static/htmx.min.js"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200, "htmx.min.js should be served");

    let body = resp.text().await.expect("body text");
    assert!(body.contains("htmx"), "response should contain htmx code");
}

#[tokio::test]
async fn admin_redirects_to_login_when_unauthenticated_docker() {
    let server = spawn_server().await;
    let base = &server.url;
    // Don't follow redirects — we want to observe the 3xx itself.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build client");

    let resp = client
        .get(format!("{base}/admin"))
        .send()
        .await
        .expect("request failed");
    assert!(
        resp.status().is_redirection(),
        "unauthenticated /admin should redirect, got: {}",
        resp.status()
    );
    let location = resp
        .headers()
        .get("location")
        .expect("redirect location")
        .to_str()
        .expect("valid header");
    assert!(
        location.starts_with("/auth/login"),
        "should redirect to /auth/login, got: {location}"
    );
}
