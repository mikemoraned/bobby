#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use clap::Parser;
use shared::RefineModels;
use skeet_publish::{
    CdnImageUrlResolver, FeedPublisher, Limit, Order, PublishMetrics, PublishOutcome,
    PublishedList, connect, parse_spec,
};
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
    let metrics = PublishMetrics::new(&opentelemetry::global::meter("skeet_publish"));

    // `--once` runs a single cycle (and on a fresh publisher the gate always
    // fires, so it publishes); the loop gates on table-version changes so an
    // idle worker does no store/redis work.
    if args.once {
        publish(&publisher, &args.redis_publish_url, &metrics).await?;
        return Ok(());
    }

    let mut interval = tokio::time::interval(Duration::from_secs(args.interval_secs));
    loop {
        interval.tick().await;
        if let Err(e) = publish(&publisher, &args.redis_publish_url, &metrics).await {
            metrics.record_failed();
            warn!(error = %e, "publish cycle failed");
        }
    }
}

/// One gated publish cycle: connect fresh (so a long-running loop never reuses a
/// connection Upstash dropped while idle), publish only if something moved, and
/// log/record what landed (the specs come from the publisher via the outcome).
async fn publish(
    publisher: &FeedPublisher,
    redis_url: &str,
    metrics: &PublishMetrics,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = connect(redis_url).await?;
    match publisher.publish_if_changed(&mut conn, Utc::now()).await? {
        PublishOutcome::Unchanged => {
            metrics.record_unchanged();
            info!("no relevant table change — skipped publish");
        }
        PublishOutcome::Published(specs) => {
            metrics.record_published();
            log_published_lists(&specs, &mut conn, metrics).await?;
        }
    }
    Ok(())
}

/// Read each list back, log a summary, and record its size — so a local run
/// shows what landed in redis and the worker exports list sizes.
async fn log_published_lists(
    specs: &[(Order, Limit)],
    conn: &mut deadpool_redis::redis::aio::MultiplexedConnection,
    metrics: &PublishMetrics,
) -> Result<(), Box<dyn std::error::Error>> {
    for (order, limit) in specs {
        let list = PublishedList::new(*order, *limit);
        let pairs = list.read(conn).await?;
        let refreshed_at = list.refreshed_at(conn).await?;
        metrics.record_list_size(&list.name(), pairs.len() as u64);
        info!(list = %list.name(), count = pairs.len(), ?refreshed_at, "published list");
        for pair in pairs.iter().take(3) {
            info!(list = %list.name(), skeet_id = %pair.skeet_id, image_url = %pair.image_url, "  sample pair");
        }
    }
    Ok(())
}
