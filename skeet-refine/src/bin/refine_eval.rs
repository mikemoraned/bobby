use std::path::PathBuf;

use clap::Parser;
use eval::{
    EvalResults, EvalSplit, LabelledScore, ModelPrices, confusion_at, pin_at_precision,
    roc_auc_score,
};
use futures::stream::{self, StreamExt};
use shared::Score;
use skeet_refine::loader::{LabelledImage, load_band_index, load_labelled_images};
use skeet_refine::model::load_model;
use skeet_refine::refining::{RefineAgent, build_agent, create_client, refine_image};
use skeet_store::StoreArgs;
use tracing::{error, info};

#[derive(Parser)]
#[command(
    name = "refine-eval",
    about = "Evaluate the configured refine model against the frozen held-out test split"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the frozen eval split written by `capture-appraisals`
    #[arg(long, default_value = "config/eval-split.toml")]
    split_path: PathBuf,

    /// Path to the refine model config under evaluation
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// Path to write the eval results
    #[arg(long)]
    output: PathBuf,

    /// OpenAI API key
    #[arg(long, env = "BOBBY_OPENAI_API_KEY")]
    openai_api_key: String,

    /// Maximum concurrent OpenAI scoring requests
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

#[derive(Debug, thiserror::Error)]
enum EvalRunError {
    #[error("no positive labels in test set — split is broken")]
    NoPositives,
    #[error("no positive predictions at the model's decision_threshold — model is broken")]
    NoPositivePredictions,
}

async fn score_all(
    agent: &RefineAgent,
    images: &[LabelledImage],
    concurrency: usize,
) -> Result<Vec<(Score, u64, u64)>, Box<dyn std::error::Error>> {
    let total = images.len();
    let scored: Vec<(Score, u64, u64)> = stream::iter(images.iter())
        .map(|labelled| async move {
            refine_image(agent, &labelled.image)
                .await
                .map(|(s, usage, _d)| (s, usage.input_tokens, usage.output_tokens))
        })
        .buffered(concurrency)
        .enumerate()
        .map(|(i, r)| {
            if let Err(e) = &r {
                error!(idx = i, error = %e, "scoring failed");
            } else if i % 10 == 0 {
                info!(idx = i, total, "scoring progress");
            }
            r
        })
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_, _>>()?;
    Ok(scored)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    info!(git_hash = env!("BUILD_GIT_HASH"), "refine-eval starting");

    let args = Args::parse();

    let split = EvalSplit::load(&args.split_path)?;
    let split_hash = split.content_hash();
    info!(
        path = %args.split_path.display(),
        hash = %split_hash,
        test_count = split.test.len(),
        "loaded frozen split"
    );

    let model = load_model(&args.model_path)?;
    let model_version = model.version();
    info!(
        path = %args.model_path.display(),
        model_name = %model.model_name,
        %model_version,
        "loaded refine model"
    );

    let store = args.store.open_store("refine-eval").await?;
    let band_by_id = load_band_index(&store).await?;
    let test_images = load_labelled_images(&store, &band_by_id, &split.test).await?;

    info!(count = test_images.len(), "fetched test images, scoring");

    let client = create_client(&args.openai_api_key);
    let agent = build_agent(&client, model.model_name.as_str(), model.prompt.as_str());
    let scored = score_all(&agent, &test_images, args.concurrency).await?;

    let (input_tokens, output_tokens): (u64, u64) = scored
        .iter()
        .fold((0u64, 0u64), |(i, o), (_, ti, to)| (i + ti, o + to));

    let labelled: Vec<LabelledScore> = test_images
        .iter()
        .zip(scored.iter())
        .map(|(img, (score, _, _))| LabelledScore {
            score: *score,
            is_positive: img.is_positive(),
        })
        .collect();

    let decision_threshold = model.decision_threshold;
    let matrix = confusion_at(&labelled, decision_threshold);
    let precision = matrix.precision().ok_or(EvalRunError::NoPositivePredictions)?;
    let recall = matrix.recall().ok_or(EvalRunError::NoPositives)?;
    let f1 = matrix.f1().expect("precision and recall both defined");
    let roc_auc = roc_auc_score(&labelled);
    let pinned_precision = pin_at_precision(&labelled, precision);

    let prices = ModelPrices::embedded()?;
    let cost_usd = prices.cost_for(model.model_name.as_str(), input_tokens, output_tokens)?;

    let results = EvalResults {
        split_config_path: args.split_path.display().to_string(),
        split_config_hash: split_hash,
        model_version: model_version.to_string(),
        model_name: model.model_name.to_string(),
        precision,
        recall,
        f1,
        roc_auc,
        pinned_precision,
        tp: matrix.true_pos,
        fp: matrix.false_pos,
        tn: matrix.true_neg,
        fn_: matrix.false_neg,
        input_tokens,
        output_tokens,
        cost_usd,
    };
    results.save(&args.output)?;

    println!();
    println!("=== Eval results ===");
    println!("  model       : {} ({})", model.model_name, model_version);
    println!("  test images : {}", test_images.len());
    println!("  precision   : {precision} (threshold {decision_threshold})");
    println!("  recall      : {recall} (threshold {decision_threshold})");
    println!("  f1          : {f1} (threshold {decision_threshold})");
    match roc_auc {
        Some(v) => println!("  roc-auc     : {v}"),
        None => println!("  roc-auc     : (undefined — only one class present)"),
    }
    match pinned_precision {
        Some(p) => println!(
            "  pinned@P={precision}: threshold={}, recall={}",
            p.threshold, p.recall
        ),
        None => println!("  pinned@P={precision}: no qualifying threshold"),
    }
    println!(
        "  tokens      : input={input_tokens}, output={output_tokens}, cost_usd={cost_usd:.4}"
    );
    println!("  written     : {}", args.output.display());

    Ok(())
}
