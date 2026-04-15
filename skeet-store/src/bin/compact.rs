#![warn(clippy::all, clippy::nursery)]

use clap::Parser;
use skeet_store::{CompactTarget, StoreArgs};
use tracing::info;

#[derive(Parser)]
#[command(about = "Force-compact a LanceDB store to reduce fragment count")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Which table(s) to compact
    #[arg(long, value_enum, default_value_t = CompactTarget::All)]
    table: CompactTarget,

    /// Only check and report storage health, don't compact
    #[arg(long)]
    check_only: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();
    let store = args.store.open_store().await?;

    let health = store.storage_health().await?;
    health.print_report();

    if args.check_only {
        return Ok(());
    }

    if !health.needs_action() {
        info!("no compaction needed, skipping");
        return Ok(());
    }

    info!(table = ?args.table, "starting compaction");
    store.compact_table(args.table).await?;
    info!("compaction finished");

    let health_after = store.storage_health().await?;
    health_after.print_report();

    Ok(())
}
