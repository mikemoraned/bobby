#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::{StoreArgs, StoreMetrics};
use tracing::info;

#[derive(Parser)]
#[command(
    about = "Optimise a LanceDB store: compact fragments, rebuild indices, prune old versions"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Only check and report storage health, don't optimise
    #[arg(long)]
    check_only: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file("info", "optimise");
    info!(git_hash = env!("BUILD_GIT_HASH"), "optimise starting");

    let args = Args::parse();
    let store = args.store.open_store("optimise").await?;
    let store_metrics = StoreMetrics::new(opentelemetry::global::meter("lance"));

    let health = store.storage_health().await?;
    health.print_report();

    if args.check_only {
        let counts = store.fragment_counts().await?;
        store_metrics.record_fragment_counts(&counts);
        return Ok(());
    }

    if health.needs_action() {
        info!("starting optimisation");
        store.optimise().await?;
        info!("optimisation finished");
        let health_after = store.storage_health().await?;
        health_after.print_report();
    } else {
        info!("no optimisation needed, skipping");
    }

    info!("pruning old versions");
    store.prune_old_versions().await?;
    info!("prune finished");

    let counts = store.fragment_counts().await?;
    store_metrics.record_fragment_counts(&counts);

    Ok(())
}
