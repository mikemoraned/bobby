use cot::http::HeaderValue;
use cot::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::UrlQuery;
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument, warn};

use crate::feed_config::FeedConfig;
use crate::{FeedSourceExtractor, PublishedImagesSourceExtractor};

/// The grid is republished at most once a publish cycle and the app suspends
/// when idle, so a short shared cache with revalidation is the right trade: a
/// burst is absorbed, yet a republish is picked up within the window.
const GRID_CACHE_CONTROL: &str = "public, max-age=60";

fn wants_no_cache(head: &RequestHead) -> bool {
    head.headers
        .get(CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("no-cache"))
}

fn set_last_modified_header(
    response: &mut Response,
    refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
) {
    if let Some(at) = refreshed_at {
        web_support::set_last_modified(response, at);
    }
}

/// Set the grid's caching headers: a short shared `cache-control` always, plus
/// `last-modified` when the backing list has a known refresh time.
fn set_cache_headers(response: &mut Response, refreshed_at: Option<chrono::DateTime<chrono::Utc>>) {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(GRID_CACHE_CONTROL));
    set_last_modified_header(response, refreshed_at);
}

fn json_response(body: &impl Serialize) -> cot::Result<Response> {
    let json = serde_json::to_string(body)
        .map_err(|e| cot::Error::internal(format!("failed to serialize JSON: {e}")))?;
    let mut response = Response::new(Body::fixed(json));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
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
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        return Ok(response);
    }

    let limit = query
        .limit
        .unwrap_or(config.max_entries)
        .min(config.max_entries);

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

/// One image card on the home page: the Bluesky post it links to and the CDN
/// thumbnail it shows. `aspect_ratio` is the `W/H` for the `aspect-ratio` CSS
/// property when the image's dimensions are known, so the tile reserves space
/// and the grid doesn't reflow as images load.
struct GridCard {
    bsky_url: String,
    thumb_url: String,
    alt: String,
    aspect_ratio: Option<String>,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    cards: Vec<GridCard>,
    /// The shared explanatory blurb shown in the banner.
    blurb: &'static str,
    /// `bsky.app` URL for subscribing to the feed.
    feed_bsky_url: String,
    /// Inline SVG QR code for the site URL; `None` if encoding failed (the
    /// banner then renders without it rather than failing the page).
    qr_svg: Option<String>,
    /// The "images examined" banner stat, pre-formatted with thousands
    /// separators (e.g. `"21,621,500"`) for display.
    examined_count: Option<String>,
    /// Site-specific Plausible analytics script URL; `Some` renders the
    /// tracking script, `None` omits it.
    plausible_script_url: Option<String>,
}

/// Group a non-negative integer with comma thousands separators, e.g.
/// `21621500` → `"21,621,500"`. Neither Rust's formatter nor askama has built-in
/// digit grouping, so the banner stat is grouped here at render time.
fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + (digits.len() - 1) / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Home page: a server-rendered grid of the published `quality-7d` images, each
/// linking through to its Bluesky post. Images are served by the Bluesky CDN, so
/// this page only renders HTML.
#[instrument(skip_all)]
pub async fn home(
    head: RequestHead,
    PublishedImagesSourceExtractor(source): PublishedImagesSourceExtractor,
    FeedConfig(config): FeedConfig,
) -> cot::Result<Response> {
    let published = source
        .published_images()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read published images: {e}")))?;

    let refreshed_at = published.refreshed_at;

    // Conditional GET: if the client already holds this revision, skip rendering.
    if let Some(at) = refreshed_at
        && let Some(not_modified) =
            web_support::not_modified_since(&head, at, Some(GRID_CACHE_CONTROL))
    {
        info!("not modified since client's copy — 304");
        return Ok(not_modified);
    }

    let cards: Vec<GridCard> = published
        .images
        .into_iter()
        .map(|item| GridCard {
            bsky_url: item.skeet_id.bsky_post_url(),
            thumb_url: item.image_url.to_string(),
            alt: "Selfie with a landmark".to_string(),
            aspect_ratio: item
                .image_url_dimensions
                .map(|d| format!("{}/{}", d.width, d.height)),
        })
        .collect();

    // The banner stat is best-effort: a missing or unreadable count must not
    // fail the page, so fold any error into "no number shown".
    let examined_count = source
        .examined_count()
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "failed to read examined count; rendering without it");
            None
        })
        .map(group_thousands);

    // The QR is best-effort decoration: if encoding ever fails, render the
    // banner without it rather than failing the whole page.
    let qr_svg = crate::qr::qr_svg(&config.site_url())
        .map_err(|e| warn!(error = %e, "failed to render site QR; rendering without it"))
        .ok();

    info!(count = cards.len(), "serving home grid");
    let rendered = HomeTemplate {
        cards,
        blurb: crate::FEED_BLURB,
        feed_bsky_url: config.feed_bsky_url(),
        qr_svg,
        examined_count,
        plausible_script_url: config.plausible_script_url.clone(),
    }
    .render()?;
    let mut response = Response::new(Body::fixed(rendered));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    set_cache_headers(&mut response, refreshed_at);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cot::http::header::LAST_MODIFIED;

    #[test]
    fn group_thousands_inserts_separators_every_three_digits() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(7), "7");
        assert_eq!(group_thousands(42), "42");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(21_621_500), "21,621,500");
    }

    #[test]
    fn wants_no_cache_true_when_header_present() {
        let req = cot::http::Request::builder()
            .header(CACHE_CONTROL, "no-cache")
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
            .header(CACHE_CONTROL, "max-age=60")
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
            .get(LAST_MODIFIED)
            .expect("header should be set")
            .to_str()
            .expect("valid str");
        assert_eq!(val, "Sat, 15 Jun 2024 09:30:00 GMT");
    }

    #[test]
    fn set_last_modified_header_noop_when_none() {
        let mut response = Response::new(Body::empty());
        set_last_modified_header(&mut response, None);
        assert!(response.headers().get(LAST_MODIFIED).is_none());
    }
}
