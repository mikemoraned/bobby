use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Extract the session cookie value from a response's Set-Cookie header.
pub fn extract_session_cookie(response: &cot::http::Response<cot::Body>) -> Option<String> {
    response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").to_string())
}

/// Build a GET request with an optional session cookie.
pub fn get_with_cookie(uri: &str, cookie: Option<&str>) -> cot::http::Request<cot::Body> {
    get_with_cookie_and_headers(uri, cookie, &[])
}

/// Build a GET request with an optional session cookie and extra headers.
pub fn get_with_cookie_and_headers(
    uri: &str,
    cookie: Option<&str>,
    extra_headers: &[(&str, &str)],
) -> cot::http::Request<cot::Body> {
    let mut builder = cot::http::Request::builder().uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    builder.body(cot::Body::empty()).expect("build request")
}

/// Extract a query parameter from a URL string.
pub fn extract_query_param(url: &str, param: &str) -> Option<String> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        if parts.next() == Some(param) {
            return parts
                .next()
                .map(|v| urlencoding::decode(v).unwrap_or_default().into_owned());
        }
    }
    None
}

/// Mount mock responses for GitHub token exchange and /user API.
pub async fn mount_github_mocks(mock_server: &MockServer, github_username: &str) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "test-access-token",
            "token_type": "bearer"
        })))
        .mount(mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "login": github_username,
        })))
        .mount(mock_server)
        .await;
}
