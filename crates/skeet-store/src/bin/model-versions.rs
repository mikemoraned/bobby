use clap::Parser;
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "model-versions",
    about = "Scan the scores table and print distinct model_version values with counts"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("warn");
    info!(git_hash = env!("BUILD_GIT_HASH"), "model-versions starting");

    let args = Args::parse();
    let store = args.store.open_store("model-versions").await?;

    let counts = store.count_scores_by_model_version().await?;

    if counts.is_empty() {
        println!("No scores found.");
        return Ok(());
    }

    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    println!("{:<20} {:>8}", "model_version", "count");
    println!("{}", "-".repeat(30));
    for (version, count) in &sorted {
        println!("{:<20} {:>8}", version, count);
    }
    Ok(())
}
