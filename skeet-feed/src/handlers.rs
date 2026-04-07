use cot::request::extractors::UrlQuery;
use cot::response::Response;
use cot::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument, warn};

use crate::Store;
use crate::feed_config::FeedConfig;

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
    Store(store): Store,
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

    let scored = store
        .list_scored_summaries_by_score(limit, Some(config.max_age_hours))
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))?;

    let posts: Vec<SkeletonFeedPost> = scored
        .into_iter()
        .filter(|(summary, score)| {
            let above_threshold = f32::from(*score) >= config.min_score;
            let is_post = summary.skeet_id.collection() == "app.bsky.feed.post";
            above_threshold && is_post
        })
        .take(limit)
        .map(|(summary, _score)| SkeletonFeedPost {
            post: summary.skeet_id.to_string(),
        })
        .collect();

    info!(count = posts.len(), "returning feed skeleton");

    let resp = FeedSkeletonResponse {
        feed: posts,
        cursor: None,
    };
    json_response(&resp)
}
