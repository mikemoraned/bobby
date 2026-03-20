#![warn(clippy::all, clippy::nursery)]

mod handlers;

use std::sync::OnceLock;

use clap::Parser;
use cot::config::ProjectConfig;
use cot::project::{Bootstrapper, RegisterAppsContext};
use cot::router::{Route, Router};
use cot::{App, AppBuilder, Project};
use skeet_store::StoreArgs;
use tracing::info;
use tracing_appender::rolling;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

use handlers::{annotated_image, feed};

static STORE_ARGS: OnceLock<StoreArgs> = OnceLock::new();

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
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
    let file_appender = rolling::daily("logs", "feed.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skeet_feed=info".parse().expect("valid filter")),
        )
        .with(fmt::layer().with_writer(non_blocking))
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    let args = Args::parse();
    STORE_ARGS
        .set(args.store)
        .expect("store args already initialized");

    info!("starting skeet-feed server on 127.0.0.1:8000");

    let bootstrapper = Bootstrapper::new(FeedProject)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, "127.0.0.1:8000").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use handlers::to_feed_entry;
    use skeet_store::{ImageId, SkeetId, Zone};

    use super::*;

    #[test]
    fn converts_at_uri_to_entry() {
        let image_id = ImageId::new();
        let skeet_id: SkeetId = "at://did:plc:abc123/app.bsky.feed.post/xyz789"
            .parse()
            .expect("valid AT URI");
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
    fn rejects_invalid_at_uri() {
        assert!("not-an-at-uri".parse::<SkeetId>().is_err());
    }

    #[test]
    fn returns_none_for_non_post_uri() {
        let image_id = ImageId::new();
        let skeet_id: SkeetId = "at://did:plc:abc123/app.bsky.feed.like/xyz789"
            .parse()
            .expect("valid AT URI");
        let zone = Zone::TopRight;
        assert!(to_feed_entry(&image_id, &skeet_id, &zone, "v1", "").is_none());
    }
}
