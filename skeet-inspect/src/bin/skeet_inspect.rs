#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use cot::project::Bootstrapper;
use skeet_inspect::StoreLayer;
use skeet_inspect::project::InspectProject;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let console = args.tokio_console_port.map_or(
        shared::tracing::TokioConsoleSupport::Disabled,
        |port| shared::tracing::TokioConsoleSupport::Enabled { port },
    );
    let _guard = shared::tracing::init_with_file_and_stderr("skeet_inspect=info,shared=info,skeet_store=info", "inspect.log", console);
    let store = args
        .store
        .open_store()
        .await
        .expect("failed to open store at startup");

    info!("starting skeet-inspect server on 127.0.0.1:8000");

    let project = InspectProject {
        store_layer: StoreLayer::new(store),
    };
    let bootstrapper = Bootstrapper::new(project)
        .with_config_name("dev")?
        .boot()
        .await?;
    cot::run(bootstrapper, "127.0.0.1:8000").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use image::DynamicImage;
    use skeet_inspect::handlers::to_feed_entry;
    use skeet_store::{DiscoveredAt, ImageId, SkeetId, Zone};

    #[test]
    fn converts_at_uri_to_entry() {
        let discovered_at = DiscoveredAt::now();
        let image_id = ImageId::from_image(&DynamicImage::new_rgba8(1, 1));
        let skeet_id: SkeetId = "at://did:plc:abc123/app.bsky.feed.post/xyz789"
            .parse()
            .expect("valid AT URI");
        let zone = Zone::TopRight;
        let entry = to_feed_entry(&discovered_at, &image_id, &skeet_id, &zone, "v1", "hello")
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
        let discovered_at = DiscoveredAt::now();
        let image_id = ImageId::from_image(&DynamicImage::new_rgba8(1, 1));
        let skeet_id: SkeetId = "at://did:plc:abc123/app.bsky.feed.like/xyz789"
            .parse()
            .expect("valid AT URI");
        let zone = Zone::TopRight;
        assert!(to_feed_entry(&discovered_at, &image_id, &skeet_id, &zone, "v1", "").is_none());
    }
}
