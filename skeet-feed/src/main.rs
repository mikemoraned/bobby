#![warn(clippy::all, clippy::nursery)]

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::OnceLock;

use clap::Parser;
use cot::config::ProjectConfig;
use cot::html::Html;
use cot::project::{Bootstrapper, RegisterAppsContext};
use cot::request::extractors::Path;
use cot::response::Response;
use cot::router::{Route, Router};
use cot::{App, AppBuilder, Body, Project, StatusCode, Template};
use skeet_store::{ImageId, SkeetId, SkeetStore, Zone};

static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store_path: PathBuf,
}

#[derive(Debug)]
struct FeedEntry {
    image_id: String,
    zone: String,
    config_version: String,
    detected_text: String,
    at_uri: String,
    web_url: String,
}

fn to_feed_entry(
    image_id: &ImageId,
    skeet_id: &SkeetId,
    zone: &Zone,
    config_version: &str,
    detected_text: &str,
) -> Option<FeedEntry> {
    let at_uri = skeet_id.as_str();
    let stripped = at_uri.strip_prefix("at://")?;
    let (did, rest) = stripped.split_once('/')?;
    let rkey = rest.strip_prefix("app.bsky.feed.post/")?;
    Some(FeedEntry {
        image_id: image_id.to_string(),
        zone: zone.to_string(),
        config_version: config_version.to_string(),
        detected_text: detected_text.to_string(),
        at_uri: at_uri.to_string(),
        web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
    })
}

#[derive(Debug, Template)]
#[template(path = "feed.html")]
struct FeedTemplate {
    entries: Vec<FeedEntry>,
}

const MAX_FEED_ENTRIES: usize = 50;

async fn feed() -> cot::Result<Html> {
    let store = open_store().await?;

    let mut summaries = store
        .list_all_summaries()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))?;

    summaries.sort_by(|a, b| b.discovered_at.cmp(&a.discovered_at));

    let entries: Vec<FeedEntry> = summaries
        .iter()
        .take(MAX_FEED_ENTRIES)
        .filter_map(|img| {
            to_feed_entry(
                &img.image_id,
                &img.skeet_id,
                &img.zone,
                img.config_version.as_str(),
                &img.detected_text,
            )
        })
        .collect();

    let template = FeedTemplate { entries };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}

async fn annotated_image(Path(image_id_str): Path<String>) -> cot::Result<Response> {
    let image_id: ImageId = image_id_str
        .parse()
        .map_err(|_| cot::Error::internal(format!("invalid image id: {image_id_str}")))?;

    let store = open_store().await?;
    let stored = store
        .get_by_id(&image_id)
        .await
        .map_err(|e| cot::Error::internal(format!("store error: {e}")))?;

    let Some(stored) = stored else {
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

async fn open_store() -> cot::Result<SkeetStore> {
    let store_path = STORE_PATH
        .get()
        .ok_or_else(|| cot::Error::internal("store path not initialized"))?;
    SkeetStore::open(store_path.to_str().expect("valid path"), vec![])
        .await
        .map_err(|e| cot::Error::internal(format!("failed to open store: {e}")))
}

struct FeedApp;

impl App for FeedApp {
    fn name(&self) -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn router(&self) -> Router {
        Router::with_urls([
            Route::with_handler_and_name("/", feed, "feed"),
            Route::with_handler_and_name(
                "/skeet/{image_id}/annotated.png",
                annotated_image,
                "annotated_image",
            ),
        ])
    }
}

struct FeedProject;

impl Project for FeedProject {
    fn config(&self, _config_name: &str) -> cot::Result<ProjectConfig> {
        Ok(ProjectConfig::dev_default())
    }

    fn register_apps(&self, apps: &mut AppBuilder, _context: &RegisterAppsContext) {
        apps.register_with_views(FeedApp, "");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    STORE_PATH
        .set(args.store_path)
        .expect("store path already initialized");

    let bootstrapper = Bootstrapper::new(FeedProject)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, "127.0.0.1:8000").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_at_uri_to_entry() {
        let image_id = ImageId::new();
        let skeet_id = SkeetId::new("at://did:plc:abc123/app.bsky.feed.post/xyz789");
        let zone = Zone::TopRight;
        let entry = to_feed_entry(&image_id, &skeet_id, &zone, "v1", "hello")
            .expect("should produce entry");
        assert_eq!(entry.at_uri, "at://did:plc:abc123/app.bsky.feed.post/xyz789");
        assert_eq!(
            entry.web_url,
            "https://bsky.app/profile/did:plc:abc123/post/xyz789"
        );
    }

    #[test]
    fn returns_none_for_invalid_uri() {
        let image_id = ImageId::new();
        let skeet_id = SkeetId::new("not-an-at-uri");
        let zone = Zone::TopRight;
        assert!(to_feed_entry(&image_id, &skeet_id, &zone, "v1", "").is_none());
    }

    #[test]
    fn returns_none_for_non_post_uri() {
        let image_id = ImageId::new();
        let skeet_id = SkeetId::new("at://did:plc:abc123/app.bsky.feed.like/xyz789");
        let zone = Zone::TopRight;
        assert!(to_feed_entry(&image_id, &skeet_id, &zone, "v1", "").is_none());
    }
}
