use std::path::PathBuf;

use clap::Parser;
use eval::{EvalSplits, PricesRegistry, SnapshotId, Usd, stratified_sample};
use futures::stream::{self, StreamExt};
use shared::refine_model::Label;
use shared::{Band, ImageId};
use skeet_refine::loader::{load_band_index, load_labelled_images};
use skeet_refine::model::RefineModels;
use skeet_refine::refining::{build_agent, create_client, refine_image_resilient};
use skeet_store::StoreArgs;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "sample-costs",
    about = "Empirically measure per-image cost for every model in a prices snapshot using a small sample from the train set"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the eval-splits registry
    #[arg(long, default_value = "config/eval-splits.toml")]
    splits_path: PathBuf,

    /// Which split to sample from (by label)
    #[arg(long, default_value = "default")]
    split_label: String,

    /// Number of images to sample from the train set
    #[arg(long, default_value_t = 10)]
    sample_size: usize,

    /// Pricing snapshot to use; defaults to the `current` label in prices.toml
    #[arg(long)]
    prices_snapshot_id: Option<SnapshotId>,

    /// Path to the refine model registry
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// OpenAI API key
    #[arg(long, env = "BOBBY_OPENAI_API_KEY")]
    openai_api_key: String,

    /// Maximum concurrent OpenAI scoring requests per model
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

struct ModelCostStats {
    model: String,
    input_per_million: Usd,
    output_per_million: Usd,
    min: Usd,
    max: Usd,
    avg: Usd,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    info!(git_hash = env!("BUILD_GIT_HASH"), "sample-costs starting");

    let args = Args::parse();

    let prices_registry = PricesRegistry::embedded()?;
    let (prices_snapshot_id, snapshot) =
        prices_registry.by_id_or_label(args.prices_snapshot_id, &Label::new("current"))?;
    info!(prices_snapshot_id = %prices_snapshot_id, "resolved prices snapshot");

    let splits = EvalSplits::load(&args.splits_path)?;
    let split_label = Label::new(&args.split_label);
    let (split_id, split) = splits.by_label(&split_label).ok_or_else(|| {
        format!(
            "split label '{}' not found in {}",
            args.split_label,
            args.splits_path.display()
        )
    })?;
    info!(%split_id, train_count = split.train.len(), "loaded split");

    let models = RefineModels::load(&args.model_path)?;
    let model = models
        .by_label(&Label::production())
        .ok_or("no production label in refine.toml")?;
    let prompt = model.prompt.as_str();
    info!(model_name = %model.model_name, "loaded production model prompt");

    let store = args.store.open_store("sample-costs").await?;
    let band_by_id = load_band_index(&store).await?;

    let train_items: Vec<(ImageId, Band)> = split
        .train
        .iter()
        .filter_map(|id| band_by_id.get(id).map(|b| (id.clone(), *b)))
        .collect();

    let sampled_ids: Vec<ImageId> = stratified_sample(&train_items, args.sample_size, 42);
    info!(count = sampled_ids.len(), "sampled train images");

    let images = load_labelled_images(&store, &band_by_id, &sampled_ids).await?;
    info!(count = images.len(), "loaded images from store, scoring each model");

    let client = create_client(&args.openai_api_key);

    let mut model_names: Vec<String> = snapshot.prices.keys().cloned().collect();
    model_names.sort_unstable();

    let mut rows: Vec<ModelCostStats> = Vec::new();

    for model_name in &model_names {
        info!(model = %model_name, "scoring sample");
        let agent = build_agent(&client, model_name, prompt);
        let agent_ref = &agent;

        let results: Vec<_> = stream::iter(images.iter())
            .map(|labelled| {
                let img = labelled.image.clone();
                async move { refine_image_resilient(agent_ref, &img).await }
            })
            .buffered(args.concurrency)
            .collect()
            .await;

        let costs: Vec<Usd> = results
            .iter()
            .map(|r| {
                snapshot
                    .cost_for(model_name, r.usage.input_tokens, r.usage.output_tokens)
                    .expect("model is in snapshot")
            })
            .collect();

        if costs.is_empty() {
            continue;
        }

        let min = *costs.iter().min().expect("non-empty");
        let max = *costs.iter().max().expect("non-empty");
        let total = costs.iter().copied().fold(Usd::zero(), |acc, c| acc + c);
        let avg = total / costs.len() as u64;

        let prices = snapshot.prices.get(model_name).expect("model is in snapshot");

        info!(
            model = %model_name,
            min = %min.round_dp(4),
            max = %max.round_dp(4),
            avg = %avg.round_dp(4),
            "model scored"
        );

        rows.push(ModelCostStats {
            model: model_name.clone(),
            input_per_million: prices.input_per_million,
            output_per_million: prices.output_per_million,
            min,
            max,
            avg,
        });
    }

    println!();
    println!(
        "Sample costs: {} images from split `{}` (train side), snapshot `{}`",
        images.len(),
        args.split_label,
        prices_snapshot_id
    );
    println!();
    println!("| model | input $/M | output $/M | min/image | max/image | avg/image |");
    println!("|---|---|---|---|---|---|");
    for row in &rows {
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            row.model,
            row.input_per_million,
            row.output_per_million,
            row.min.round_dp(4),
            row.max.round_dp(4),
            row.avg.round_dp(4),
        );
    }

    Ok(())
}
