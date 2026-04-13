use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;

use cot::html::Html;
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::{Path, UrlQuery};
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use serde::{Deserialize, Serialize};
use shared::Band;
use skeet_store::{ImageId, Score, SkeetId, StoredImageSummary};
use skeet_web_shared::Store;
use skeet_web_shared::effective_band::{image_effective_band, skeet_visible_in_feed};
use tracing::{info, instrument, warn};

use crate::FeedCacheExtractor;
use crate::feed_cache::{CachedFeed, FeedCache};
use crate::feed_config::FeedConfig;

/// Compute the set of skeet IDs whose effective band makes them visible in the feed.
fn visible_skeet_ids(feed: &CachedFeed) -> HashSet<SkeetId> {
    // Group images by skeet, computing each image's effective band.
    let mut skeet_images: HashMap<&SkeetId, Vec<Band>> = HashMap::new();
    for (summary, score) in &feed.entries {
        let manual_image = feed.image_appraisals.get(&summary.image_id).map(|a| a.band);
        let effective = image_effective_band(*score, manual_image);
        skeet_images
            .entry(&summary.skeet_id)
            .or_default()
            .push(effective);
    }

    skeet_images
        .into_iter()
        .filter(|(skeet_id, image_bands)| {
            let manual_skeet = feed.skeet_appraisals.get(skeet_id).map(|a| a.band);
            skeet_visible_in_feed(manual_skeet, image_bands)
        })
        .map(|(skeet_id, _)| skeet_id.clone())
        .collect()
}

/// Return scored entries filtered to only those from visible skeets,
/// sorted best-to-worst by score, deduplicated by skeet_id.
pub fn visible_entries(feed: &CachedFeed) -> Vec<(StoredImageSummary, Score)> {
    let visible = visible_skeet_ids(feed);

    let mut seen = HashSet::new();
    feed.entries
        .iter()
        .filter(|(summary, _)| {
            summary.skeet_id.collection() == "app.bsky.feed.post"
                && visible.contains(&summary.skeet_id)
        })
        .filter(|(summary, _)| seen.insert(summary.skeet_id.clone()))
        .cloned()
        .collect()
}

fn wants_no_cache(head: &RequestHead) -> bool {
    head.headers
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("no-cache"))
}

async fn get_feed(cache: &Arc<FeedCache>, head: &RequestHead) -> cot::Result<CachedFeed> {
    if wants_no_cache(head) {
        info!("cache-control: no-cache — forcing refresh");
        cache
            .refresh()
            .await
            .map_err(|e| cot::Error::internal(format!("failed to refresh cache: {e}")))
    } else {
        cache
            .get()
            .await
            .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))
    }
}

fn set_date_header(response: &mut Response, refreshed_at: Option<chrono::DateTime<chrono::Utc>>) {
    if let Some(at) = refreshed_at {
        let date = at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        if let Ok(val) = date.parse() {
            response.headers_mut().insert("date", val);
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
    FeedCacheExtractor(cache): FeedCacheExtractor,
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

    let feed = get_feed(&cache, &head).await?;

    let posts: Vec<SkeletonFeedPost> = visible_entries(&feed)
        .into_iter()
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
    let mut response = json_response(&resp)?;
    set_date_header(&mut response, cache.refreshed_at().await);
    Ok(response)
}

pub struct HomeEntry {
    pub image_id: String,
    pub score: String,
    pub band: String,
    pub web_url: String,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    entries: Vec<HomeEntry>,
}

#[instrument(skip_all)]
pub async fn home(
    head: RequestHead,
    FeedCacheExtractor(cache): FeedCacheExtractor,
) -> cot::Result<Html> {
    info!("serving home");

    let feed = get_feed(&cache, &head).await?;

    let entries: Vec<HomeEntry> = visible_entries(&feed)
        .into_iter()
        .filter_map(|(summary, score)| {
            if summary.skeet_id.collection() != "app.bsky.feed.post" {
                return None;
            }
            let did = summary.skeet_id.did();
            let rkey = summary.skeet_id.rkey();
            let manual_image = feed.image_appraisals.get(&summary.image_id).map(|a| a.band);
            let band = image_effective_band(score, manual_image);
            Some(HomeEntry {
                image_id: summary.image_id.to_string(),
                score: format!("{score}"),
                band: band.to_string(),
                web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
            })
        })
        .collect();

    info!(count = entries.len(), "serving home entries");
    let rendered = HomeTemplate { entries }.render()?;
    Ok(Html::new(rendered))
}

#[instrument(skip_all, fields(image_id = %image_id_str))]
pub async fn annotated_image(
    Store(store): Store,
    Path(image_id_str): Path<String>,
) -> cot::Result<Response> {
    info!(image_id = %image_id_str, "serving annotated image");
    let image_id: ImageId = image_id_str
        .parse()
        .map_err(|_| cot::Error::internal(format!("invalid image id: {image_id_str}")))?;

    let stored = store
        .get_by_id(&image_id)
        .await
        .map_err(|e| cot::Error::internal(format!("store error: {e}")))?;

    let Some(stored) = stored else {
        warn!(image_id = %image_id_str, "image not found");
        let mut response = Response::new(Body::fixed("not found"));
        *response.status_mut() = StatusCode::NOT_FOUND;
        return Ok(response);
    };

    let mut buf = Cursor::new(Vec::new());
    stored
        .annotated_image
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| cot::Error::internal(format!("failed to encode image: {e}")))?;

    let mut response = Response::new(Body::fixed(buf.into_inner()));
    response
        .headers_mut()
        .insert("content-type", "image/png".parse().expect("valid header"));
    Ok(response)
}
