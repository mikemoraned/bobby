use std::path::PathBuf;

use clap::Parser;
use eval::{EvalResults, EvalSplit, ModelPrices, Usd};
use skeet_refine::model::{Label, RefineModels};
use skeet_refine::train::gate::GateOutcome;
use skeet_refine::train::{TrainError, TrainingInputs, run_training};
use skeet_store::StoreArgs;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "train", about = "Train a scoring prompt against the wider appraised dataset")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the frozen eval split written by `capture-appraisals`
    #[arg(long, default_value = "config/eval-split.toml")]
    split_path: PathBuf,

    /// Path to the baseline eval results this run must not regress against
    #[arg(long, default_value = "config/eval-results-baseline.toml")]
    baseline_path: PathBuf,

    /// Path to write the resulting refine.toml — only written if the acceptance gate accepts
    #[arg(long, default_value = "config/refine.toml")]
    model_output: PathBuf,

    /// Path to write this run's eval results (always written, regardless of gate outcome)
    #[arg(long)]
    eval_output: PathBuf,

    /// OpenAI API key
    #[arg(long, env = "BOBBY_OPENAI_API_KEY")]
    openai_api_key: String,

    /// Maximum training iterations
    #[arg(long, default_value_t = 10)]
    max_iterations: u32,

    /// Approximate dollar budget for the entire training run, including the
    /// final full-test-set evaluation. Per-iteration sample size is derived
    /// from this budget and the baseline's per-image scoring cost.
    #[arg(long = "budget-usd", default_value = "5.0")]
    budget: Usd,

    /// OpenAI model name to use for both scoring and prompt rewriting
    #[arg(long, default_value = "gpt-4o")]
    model: String,

    /// Maximum concurrent OpenAI scoring requests
    #[arg(long, default_value_t = 4)]
    concurrency: usize,

    /// Random seed for per-iteration subsampling
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
    info!(git_hash = env!("BUILD_GIT_HASH"), "train starting");

    let args = Args::parse();

    let mut models = RefineModels::load_or_empty(&args.model_output)?;
    info!(path = %args.model_output.display(), "loaded refine models");

    let split = EvalSplit::load(&args.split_path)?;
    let baseline = EvalResults::load(&args.baseline_path)?;
    let split_hash = split.content_hash();
    if split_hash != baseline.split_config_hash {
        return Err(TrainError::SplitHashDrift {
            split_hash,
            baseline_hash: baseline.split_config_hash,
        }
        .into());
    }
    info!(
        split_path = %args.split_path.display(),
        baseline_path = %args.baseline_path.display(),
        train_count = split.train.len(),
        test_count = split.test.len(),
        hash = %split_hash,
        "loaded split + baseline (hash matches)"
    );

    let prices = ModelPrices::embedded()?;
    let store = args.store.open_store("train").await?;

    let inputs = TrainingInputs {
        store: &store,
        split: &split,
        split_path_str: args.split_path.display().to_string(),
        split_hash,
        baseline: &baseline,
        prices: &prices,
        openai_api_key: &args.openai_api_key,
        max_iterations: args.max_iterations,
        budget: args.budget,
        model: args.model.clone(),
        concurrency: args.concurrency,
        seed: args.seed,
    };

    let report = run_training(inputs).await?;
    report.results.save(&args.eval_output)?;

    print_metrics(&args, &baseline, &report);

    match report.outcome {
        GateOutcome::Accepted => {
            models.insert(report.candidate_model.clone(), &[Label::production()]);
            models.save(&args.model_output)?;
            print_accepted(&args, &baseline, &report);
            info!(
                path = %args.model_output.display(),
                decision_threshold = %report.candidate_model.decision_threshold,
                "saved new refine.toml"
            );
        }
        GateOutcome::Rejected => {
            print_rejected(&args, &baseline);
            warn!("acceptance gate rejected candidate; refine.toml left untouched");
        }
    }

    if report.total_cost > args.budget {
        let overshoot = report.total_cost - args.budget;
        let pct = overshoot.ratio_as_f64(args.budget) * 100.0;
        println!();
        println!(
            "  BUDGET OVERSHOOT   : total {} exceeds --budget-usd {} by {overshoot} ({pct:.1}%)",
            report.total_cost, args.budget,
        );
        warn!(
            total_cost = %report.total_cost,
            budget = %args.budget,
            overshoot = %overshoot,
            overshoot_pct = pct,
            "training run exceeded budget"
        );
    }

    Ok(())
}

fn print_metrics(args: &Args, baseline: &EvalResults, report: &skeet_refine::train::TrainingReport) {
    let candidate_version = report.candidate_model.version();
    let results = &report.results;

    println!();
    println!("=== Training results ===");
    println!("  model              : {} ({})", args.model, candidate_version);
    println!("  iterations         : {}", args.max_iterations);
    println!("  per-iter sample    : {}", report.per_iter_size);
    if let Some(best) = report.best_train_f1 {
        println!("  best train F1      : {best}");
    } else {
        println!("  best train F1      : (undefined in every iteration)");
    }
    println!("  test precision     : {}", results.precision);
    println!("  test recall        : {}", results.recall);
    println!("  test F1            : {}", results.f1);
    match results.roc_auc {
        Some(v) => println!("  test ROC-AUC       : {v}"),
        None => println!("  test ROC-AUC       : (undefined — only one class present)"),
    }
    println!(
        "  baseline precision : {} (recall {})",
        baseline.precision, baseline.recall
    );
    match results.pinned_precision {
        Some(p) => println!(
            "  pinned@baseline P  : threshold={}, recall={}",
            p.threshold, p.recall
        ),
        None => println!("  pinned@baseline P  : no qualifying threshold"),
    }
    println!(
        "  test cost          : {}  (total run incl. iterations: {})",
        results.cost, report.total_cost
    );
    println!("  written eval       : {}", args.eval_output.display());
}

fn print_accepted(args: &Args, baseline: &EvalResults, report: &skeet_refine::train::TrainingReport) {
    println!();
    println!(
        "  ACCEPTED: candidate clears baseline precision={} with recall ≥ {} - {}",
        baseline.precision,
        baseline.recall,
        skeet_refine::train::gate::GATE_RECALL_TOLERANCE,
    );
    println!(
        "  saved decision_threshold : {}",
        report.candidate_model.decision_threshold
    );
    println!("  written model      : {}", args.model_output.display());
}

fn print_rejected(args: &Args, baseline: &EvalResults) {
    println!();
    println!(
        "  REJECTED: candidate did not match baseline precision={} at recall ≥ {} - {}",
        baseline.precision,
        baseline.recall,
        skeet_refine::train::gate::GATE_RECALL_TOLERANCE,
    );
    println!(
        "  refine.toml at {} left untouched",
        args.model_output.display()
    );
}
