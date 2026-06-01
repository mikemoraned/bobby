#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use clap::Parser;
use shared::RefineModels;
use skeet_publish::{CdnImageUrlResolver, FeedPublisher, Limit, Order, PublishedList, connect};
use skeet_store::StoreArgs;
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "skeet-publish",
    about = "Compute feed lists from the store and publish them to the redis publish server"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the refine model registry (refine.toml)
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// Redis URL for the publish server (env: BOBBY_REDIS_PUBLISH_URL)
    #[arg(long, env = "BOBBY_REDIS_PUBLISH_URL")]
    redis_publish_url: String,

    /// A list to publish, as `<order>-<limit>` (e.g. `recency-48h`). Repeatable.
    #[arg(long = "publish", default_value = "recency-48h")]
    publish: Vec<String>,

    /// Polling interval in seconds (ignored with `--once`)
    #[arg(long, default_value_t = 60)]
    interval_secs: u64,

    /// Publish a single cycle and exit (for local verification)
    #[arg(long, default_value_t = false)]
    once: bool,
}

fn parse_spec(s: &str) -> Result<(Order, Limit), String> {
    let (order, limit) = s
        .split_once('-')
        .ok_or_else(|| format!("expected <order>-<limit>, got {s:?}"))?;
    let order: Order = order.parse().map_err(|e| format!("{e}"))?;
    let limit: Limit = limit.parse().map_err(|e| format!("{e}"))?;
    Ok((order, limit))
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

    let _guard = shared::tracing::init_with_file(
        "skeet_publish=info,shared=info,skeet_store=info",
        "skeet-publish.log",
    );

    let specs: Vec<(Order, Limit)> = args
        .publish
        .iter()
        .map(|s| parse_spec(s))
        .collect::<Result<_, _>>()?;
    info!(publish = ?args.publish, interval_secs = args.interval_secs, once = args.once, "skeet-publish starting");

    let store = Arc::new(args.store.open_store("skeet_publish").await?);
    let models = Arc::new(RefineModels::load(&args.model_path)?);

    let publisher = FeedPublisher::new(
        Arc::clone(&store),
        models,
        Arc::new(CdnImageUrlResolver),
        specs.clone(),
    );

    if args.once {
        publish_cycle(&publisher, &specs, &args.redis_publish_url).await?;
        return Ok(());
    }

    let mut interval = tokio::time::interval(Duration::from_secs(args.interval_secs));
    loop {
        interval.tick().await;
        if let Err(e) = publish_cycle(&publisher, &specs, &args.redis_publish_url).await {
            warn!(error = %e, "publish cycle failed");
        }
    }
}

/// Publish every spec once, then read each list back and log a summary so a
/// local run shows what landed in redis. Connects fresh so a long-running loop
/// never reuses a connection Upstash dropped while idle.
async fn publish_cycle(
    publisher: &FeedPublisher,
    specs: &[(Order, Limit)],
    redis_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = connect(redis_url).await?;
    publisher.publish(&mut conn, Utc::now()).await?;

    for (order, limit) in specs {
        let list = PublishedList::new(*order, *limit);
        let pairs = list.read(&mut conn).await?;
        let refreshed_at = list.refreshed_at(&mut conn).await?;
        info!(list = %list.name(), count = pairs.len(), ?refreshed_at, "published list");
        for pair in pairs.iter().take(3) {
            info!(list = %list.name(), skeet_id = %pair.skeet_id, image_url = %pair.image_url, "  sample pair");
        }
    }
    Ok(())
}
