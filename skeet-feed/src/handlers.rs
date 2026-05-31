use cot::html::Html;
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::UrlQuery;
use cot::response::Response;
use cot::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument, warn};

use crate::FeedSourceExtractor;
use crate::feed_config::FeedConfig;

fn wants_no_cache(head: &RequestHead) -> bool {
    head.headers
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("no-cache"))
}

fn set_last_modified_header(
    response: &mut Response,
    refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
) {
    if let Some(at) = refreshed_at {
        let date = at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        if let Ok(val) = date.parse() {
            response.headers_mut().insert("last-modified", val);
        }
    }
}

fn json_response(body: &impl Serialize) -> cot::Result<Response> {
    let json = serde_json::to_string(body)
        .map_err(|e| cot::Error::internal(format!("failed to serialize JSON: {e}")))?;
    let mut response = Response::new(Body::fixed(json));
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().expect("valid header"));
    Ok(response)
}

#[derive(Serialize)]
struct DidDocument {
    #[serde(rename = "@context")]
    context: Vec<String>,
    id: String,
    service: Vec<DidService>,
}

#[derive(Serialize)]
struct DidService {
    id: String,
    #[serde(rename = "type")]
    service_type: String,
    #[serde(rename = "serviceEndpoint")]
    service_endpoint: String,
}

#[instrument(skip_all)]
pub async fn did_document(FeedConfig(config): FeedConfig) -> cot::Result<Response> {
    info!("serving /.well-known/did.json");
    let doc = DidDocument {
        context: vec!["https://www.w3.org/ns/did/v1".to_string()],
        id: config.did(),
        service: vec![DidService {
            id: "#bsky_fg".to_string(),
            service_type: "BskyFeedGenerator".to_string(),
            service_endpoint: config.service_endpoint(),
        }],
    };
    json_response(&doc)
}

#[derive(Serialize)]
struct DescribeResponse {
    did: String,
    feeds: Vec<DescribeFeed>,
}

#[derive(Serialize)]
struct DescribeFeed {
    uri: String,
}

#[instrument(skip_all)]
pub async fn describe_feed_generator(FeedConfig(config): FeedConfig) -> cot::Result<Response> {
    info!("serving describeFeedGenerator");
    let resp = DescribeResponse {
        did: config.did(),
        feeds: vec![DescribeFeed {
            uri: config.feed_uri(),
        }],
    };
    json_response(&resp)
}

#[derive(Deserialize)]
pub struct FeedSkeletonQuery {
    pub feed: String,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Serialize)]
struct FeedSkeletonResponse {
    feed: Vec<SkeletonFeedPost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

#[derive(Serialize)]
struct SkeletonFeedPost {
    post: String,
}

#[instrument(skip_all, fields(feed = %query.feed))]
pub async fn get_feed_skeleton(
    head: RequestHead,
    FeedSourceExtractor(source): FeedSourceExtractor,
    FeedConfig(config): FeedConfig,
    UrlQuery(query): UrlQuery<FeedSkeletonQuery>,
) -> cot::Result<Response> {
    info!(feed = %query.feed, cursor = ?query.cursor, limit = ?query.limit, "serving getFeedSkeleton");

    if query.feed != config.feed_uri() {
        warn!(requested = %query.feed, expected = %config.feed_uri(), "unknown feed requested");
        let mut response = Response::new(Body::fixed(
            r#"{"error":"UnknownFeed","message":"unknown feed"}"#,
        ));
        *response.status_mut() = StatusCode::BAD_REQUEST;
        response
            .headers_mut()
            .insert("content-type", "application/json".parse().expect("valid header"));
        return Ok(response);
    }

    let limit = query.limit.unwrap_or(config.max_entries).min(config.max_entries);

    let force_refresh = wants_no_cache(&head);
    if force_refresh {
        info!("cache-control: no-cache — forcing refresh");
    }
    let skeleton = source
        .skeleton(force_refresh)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read feed source: {e}")))?;

    let posts: Vec<SkeletonFeedPost> = skeleton
        .skeet_ids
        .into_iter()
        .take(limit)
        .map(|skeet_id| SkeletonFeedPost {
            post: skeet_id.to_string(),
        })
        .collect();

    info!(count = posts.len(), "returning feed skeleton");

    let resp = FeedSkeletonResponse {
        feed: posts,
        cursor: None,
    };
    let mut response = json_response(&resp)?;
    set_last_modified_header(&mut response, skeleton.refreshed_at);
    Ok(response)
}

/// Minimal placeholder home page.
///
/// Phase 5 replaces this with the public image grid; until then the feed
/// service's root just needs to be a valid page rather than a 404.
#[instrument(skip_all)]
pub async fn home() -> cot::Result<Html> {
    Ok(Html::new(
        "<!doctype html><html><head><title>bobby</title></head>\
         <body><p>Selfies with landmarks, found by Bobby.</p></body></html>",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wants_no_cache_true_when_header_present() {
        let req = cot::http::Request::builder()
            .header("cache-control", "no-cache")
            .body(())
            .expect("build");
        let (head, _) = req.into_parts();
        assert!(wants_no_cache(&head));
    }

    #[test]
    fn wants_no_cache_false_when_header_absent() {
        let req = cot::http::Request::builder().body(()).expect("build");
        let (head, _) = req.into_parts();
        assert!(!wants_no_cache(&head));
    }

    #[test]
    fn wants_no_cache_false_for_other_directives() {
        let req = cot::http::Request::builder()
            .header("cache-control", "max-age=60")
            .body(())
            .expect("build");
        let (head, _) = req.into_parts();
        assert!(!wants_no_cache(&head));
    }

    #[test]
    fn set_last_modified_header_adds_header() {
        use chrono::TimeZone as _;
        let dt = chrono::Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        let mut response = Response::new(Body::empty());
        set_last_modified_header(&mut response, Some(dt));
        let val = response
            .headers()
            .get("last-modified")
            .expect("header should be set")
            .to_str()
            .expect("valid str");
        assert_eq!(val, "Sat, 15 Jun 2024 09:30:00 GMT");
    }

    #[test]
    fn set_last_modified_header_noop_when_none() {
        let mut response = Response::new(Body::empty());
        set_last_modified_header(&mut response, None);
        assert!(response.headers().get("last-modified").is_none());
    }
}
