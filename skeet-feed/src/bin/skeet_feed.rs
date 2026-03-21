#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use cot::config::ProjectConfig;
use cot::project::{Bootstrapper, RegisterAppsContext};
use cot::router::{Route, Router};
use cot::{App, AppBuilder, Project};
use skeet_feed::handlers::{annotated_image, feed};
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Enable tokio-console on this port
    #[arg(long)]
    tokio_console_port: Option<u16>,
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

    let console = args.tokio_console_port.map_or(
        shared::tracing::TokioConsoleSupport::Disabled,
        |port| shared::tracing::TokioConsoleSupport::Enabled { port },
    );
    let _guard = shared::tracing::init_with_file_and_stderr("skeet_feed=info,shared=info,skeet_store=info", "feed.log", console);
    skeet_feed::STORE_ARGS
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
    use skeet_feed::handlers::to_feed_entry;
    use skeet_store::{ImageId, SkeetId, Zone};

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
