#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::OnceLock;

use clap::Parser;
use cot::config::ProjectConfig;
use cot::html::Html;
use cot::project::{Bootstrapper, RegisterAppsContext};
use cot::router::{Route, Router};
use cot::{App, AppBuilder, Project, Template};
use skeet_store::{SkeetId, SkeetStore};

static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store_path: PathBuf,
}

#[derive(Debug)]
struct SkeetEmbed {
    at_uri: String,
    web_url: String,
}

fn skeet_embed(skeet_id: &SkeetId) -> Option<SkeetEmbed> {
    let at_uri = skeet_id.as_str();
    let stripped = at_uri.strip_prefix("at://")?;
    let (did, rest) = stripped.split_once('/')?;
    let rkey = rest.strip_prefix("app.bsky.feed.post/")?;
    Some(SkeetEmbed {
        at_uri: at_uri.to_string(),
        web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
    })
}

#[derive(Debug, Template)]
#[template(path = "feed.html")]
struct FeedTemplate {
    skeets: Vec<SkeetEmbed>,
}

async fn feed() -> cot::Result<Html> {
    let store_path = STORE_PATH
        .get()
        .ok_or_else(|| cot::Error::internal("store path not initialized"))?;
    let store = SkeetStore::open(store_path)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to open store: {e}")))?;

    let skeet_ids = store
        .unique_skeet_ids()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))?;

    let skeets: Vec<SkeetEmbed> = skeet_ids.iter().filter_map(skeet_embed).collect();

    let template = FeedTemplate { skeets };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}

struct FeedApp;

impl App for FeedApp {
    fn name(&self) -> &'static str {
        env!("CARGO_PKG_NAME")
    }

    fn router(&self) -> Router {
        Router::with_urls([Route::with_handler_and_name("/", feed, "feed")])
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
    fn converts_at_uri_to_embed() {
        let id = SkeetId::new("at://did:plc:abc123/app.bsky.feed.post/xyz789");
        let embed = skeet_embed(&id).expect("should produce embed");
        assert_eq!(embed.at_uri, "at://did:plc:abc123/app.bsky.feed.post/xyz789");
        assert_eq!(embed.web_url, "https://bsky.app/profile/did:plc:abc123/post/xyz789");
    }

    #[test]
    fn returns_none_for_invalid_uri() {
        let id = SkeetId::new("not-an-at-uri");
        assert!(skeet_embed(&id).is_none());
    }

    #[test]
    fn returns_none_for_non_post_uri() {
        let id = SkeetId::new("at://did:plc:abc123/app.bsky.feed.like/xyz789");
        assert!(skeet_embed(&id).is_none());
    }
}
