#![warn(clippy::all, clippy::nursery)]

//! Regenerate the committed social-media preview fallback image from the current
//! published data, saving it as a PNG. The feed serves this when there are no
//! usable tiles. Re-run it whenever the montage styles change so the committed
//! image stays in sync with the live look.

use clap::Parser;
use skeet_feed::preview::selection::{MONTAGE_TILE_COUNT, select_tiles};
use skeet_feed::preview::tiles::{HttpTileSource, TileFetcher};
use skeet_feed::preview::{PREVIEW_HEIGHT, PREVIEW_WIDTH, generate_montage};
use skeet_publish::{FallbackFeedSource, Limit, Order, PublishedImagesSource};

#[derive(Parser)]
struct Args {
    /// Redis URL for the publish server (env: BOBBY_REDIS_PUBLISH_URL)
    #[arg(long, env = "BOBBY_REDIS_PUBLISH_URL")]
    redis_publish_url: String,

    /// Where to write the fallback PNG.
    #[arg(long, default_value = "crates/skeet-feed/assets/preview-fallback.png")]
    output: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The Upstash publish url is `rediss://`, so TLS runs through rustls — install
    // the process-global crypto provider once before any connection is made.
    #[allow(clippy::expect_used)]
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let args = Args::parse();

    let source = FallbackFeedSource::new(args.redis_publish_url, Order::Quality, Limit::weeks(4));
    let published = source.published_images().await?;
    let selection = select_tiles(&published.images);
    if selection.tile_urls.is_empty() {
        return Err(
            "no live tiles in the current published data; refusing to write an empty fallback"
                .into(),
        );
    }

    let fetcher = TileFetcher::new(
        HttpTileSource::new(reqwest::Client::new()),
        MONTAGE_TILE_COUNT as u64,
    );
    let png = generate_montage(
        &fetcher,
        selection.tile_urls,
        (PREVIEW_WIDTH, PREVIEW_HEIGHT),
    )
    .await?;
    std::fs::write(&args.output, png.as_slice())?;
    println!("wrote {} ({} bytes)", args.output, png.len());
    Ok(())
}
