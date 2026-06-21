use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use eval::{EvalSplit, EvalSplits, stratified_split};
use shared::refine_model::Label;
use shared::{Band, ImageId};
use skeet_store::{AppraisalSource, StoreArgs};
use tracing::info;

#[derive(Parser)]
#[command(about = "Capture a frozen train/test split from current image appraisals")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the eval-splits registry to append into (created if missing)
    #[arg(long, default_value = "config/eval-splits.toml")]
    splits_path: PathBuf,

    /// Label to assign to the new split (any previous holder of this label
    /// keeps its entry but loses the label)
    #[arg(long, default_value = "default")]
    label: String,

    /// Fraction of appraisals to place in the training set
    #[arg(long, default_value_t = 0.8)]
    train_ratio: f64,

    /// Random seed for reproducibility
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let store = args.store.open_store("capture-appraisals").await?;

    let appraisals = store.image_appraisals().list_all().await?;
    info!(count = appraisals.len(), "loaded image appraisals");

    let items: Vec<(ImageId, Band)> = appraisals
        .into_iter()
        .map(|(id, appraisal)| (id, appraisal.band))
        .collect();

    let (train, test) = stratified_split(&items, args.train_ratio, args.seed);
    info!(train = train.len(), test = test.len(), "split complete");

    let split = EvalSplit {
        seed: args.seed,
        captured_at: Utc::now(),
        train,
        test,
    };
    let label = Label::new(args.label);

    let mut splits = EvalSplits::load_or_empty(&args.splits_path)?;
    let split_id = splits.insert(split, std::slice::from_ref(&label));
    splits.save(&args.splits_path)?;
    info!(
        path = %args.splits_path.display(),
        %split_id,
        %label,
        "appended split to eval-splits registry"
    );

    Ok(())
}
