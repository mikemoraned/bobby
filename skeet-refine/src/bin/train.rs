use std::path::PathBuf;

use chrono::Utc;
use clap::Parser;
use eval::{EvalResultsLog, EvalSplits, PricesRegistry, Purpose, RunId, RunRecord, SnapshotId, Usd};
use shared::refine_model::Label;
use skeet_refine::model::RefineModels;
use skeet_refine::train::gate::GateOutcome;
use skeet_refine::train::TrainingInputs;
use skeet_store::StoreArgs;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "train", about = "Train a scoring prompt against the wider appraised dataset")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to the eval-splits registry
    #[arg(long, default_value = "config/eval-splits.toml")]
    splits_path: PathBuf,

    /// Which split (by label) to train against
    #[arg(long, default_value = "default")]
    split_label: String,

    /// Path to the eval-results log; this run's results are always appended,
    /// regardless of gate outcome
    #[arg(long, default_value = "config/eval-results.toml")]
    eval_results_path: PathBuf,

    /// Pricing snapshot id under which to cost this run. Defaults to whatever
    /// the prices registry's `current` label points at when the binary starts.
    #[arg(long)]
    prices_snapshot_id: Option<SnapshotId>,

    /// Specific baseline run to compare this run against; defaults to the
    /// best-F1 run in the log that has the production model_version and the
    /// same split_id as this run
    #[arg(long)]
    baseline_run_id: Option<RunId>,

    /// Path to write the resulting refine.toml — only written if the acceptance gate accepts
    #[arg(long, default_value = "config/refine.toml")]
    model_output: PathBuf,

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

    /// Free-text purpose for this run (e.g. "phase-4 gpt-4o-mini #1")
    #[arg(long)]
    purpose: String,
}

#[derive(Debug, thiserror::Error)]
enum TrainCliError {
    #[error("split label {0} not found in {1}")]
    UnknownSplitLabel(String, PathBuf),
    #[error("no production label in {0} — cannot select a default baseline")]
    NoProductionModel(PathBuf),
    #[error(
        "no run in {path} matches model_version {model_version} and split_id {split_id} — pass --baseline-run-id to override"
    )]
    NoBaselineCandidate {
        path: PathBuf,
        model_version: String,
        split_id: String,
    },
    #[error("baseline run_id {0} not found in {1}")]
    UnknownBaselineRunId(RunId, PathBuf),
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

    let splits = EvalSplits::load(&args.splits_path)?;
    let split_label = Label::new(&args.split_label);
    let (split_id, split) = splits.by_label(&split_label).ok_or_else(|| {
        TrainCliError::UnknownSplitLabel(args.split_label.clone(), args.splits_path.clone())
    })?;
    info!(
        path = %args.splits_path.display(),
        %split_id,
        train_count = split.train.len(),
        test_count = split.test.len(),
        "loaded split"
    );

    let mut log = EvalResultsLog::load(&args.eval_results_path)?;
    let baseline = pick_baseline(&log, &models, &args, split_id)?.clone();
    info!(
        run_id = %baseline.run_id,
        model_version = %baseline.model_version,
        precision = %baseline.evaluation.precision,
        recall = %baseline.evaluation.recall,
        "selected baseline run"
    );

    let prices_registry = PricesRegistry::embedded()?;
    let (prices_snapshot_id, prices) =
        prices_registry.by_id_or_label(args.prices_snapshot_id, &Label::new("current"))?;
    info!(prices_snapshot_id = %prices_snapshot_id, "resolved prices snapshot");

    let store = args.store.open_store("train").await?;

    let run_at = Utc::now();
    let inputs = TrainingInputs {
        store: &store,
        split,
        split_id: *split_id,
        baseline: &baseline,
        prices,
        prices_snapshot_id,
        openai_api_key: &args.openai_api_key,
        max_iterations: args.max_iterations,
        budget: args.budget,
        model: args.model.clone(),
        concurrency: args.concurrency,
        seed: args.seed,
        purpose: Purpose::new(args.purpose.clone()),
        run_at,
    };

    let report = inputs.train().await?;

    log.append(report.run.clone())?;
    log.save(&args.eval_results_path)?;

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

    let total_cost = report.run.total_cost();
    if total_cost > args.budget {
        let overshoot = total_cost - args.budget;
        let pct = overshoot.ratio_as_f64(args.budget) * 100.0;
        println!();
        println!(
            "  BUDGET OVERSHOOT   : total {total_cost} exceeds --budget-usd {} by {overshoot} ({pct:.1}%)",
            args.budget,
        );
        warn!(
            total_cost = %total_cost,
            budget = %args.budget,
            overshoot = %overshoot,
            overshoot_pct = pct,
            "training run exceeded budget"
        );
    }

    Ok(())
}

fn pick_baseline<'a>(
    log: &'a EvalResultsLog,
    models: &RefineModels,
    args: &Args,
    split_id: &eval::SplitId,
) -> Result<&'a RunRecord, TrainCliError> {
    if let Some(run_id) = args.baseline_run_id {
        return log
            .runs()
            .iter()
            .find(|r| r.run_id == run_id)
            .ok_or_else(|| {
                TrainCliError::UnknownBaselineRunId(
                    run_id,
                    args.eval_results_path.clone(),
                )
            });
    }
    let production = models
        .by_label(&Label::production())
        .ok_or_else(|| TrainCliError::NoProductionModel(args.model_output.clone()))?;
    let prod_version = production.version();
    log.best_by(|r| {
        if r.model_version == prod_version && &r.split_id == split_id {
            Some(r.evaluation.f1)
        } else {
            None
        }
    })
    .ok_or_else(|| TrainCliError::NoBaselineCandidate {
        path: args.eval_results_path.clone(),
        model_version: prod_version.to_string(),
        split_id: split_id.to_string(),
    })
}

fn print_metrics(args: &Args, baseline: &RunRecord, report: &skeet_refine::train::TrainingReport) {
    let candidate_version = report.candidate_model.version();
    let run = &report.run;

    println!();
    println!("=== Training results ===");
    println!("  run_id             : {}", run.run_id);
    println!("  purpose            : {}", run.purpose);
    println!("  model              : {} ({})", args.model, candidate_version);
    println!("  iterations         : {}", args.max_iterations);
    println!("  per-iter sample    : {}", report.per_iter_size);
    println!(
        "  fallbacks          : training={}, test={} (score=0.0 substitutions after exhausted retries)",
        report.training_fallbacks, report.test_fallbacks,
    );
    println!("  test precision     : {}", run.evaluation.precision);
    println!("  test recall        : {}", run.evaluation.recall);
    println!("  test F1            : {}", run.evaluation.f1);
    match run.evaluation.roc_auc {
        Some(v) => println!("  test ROC-AUC       : {v}"),
        None => println!("  test ROC-AUC       : (undefined — only one class present)"),
    }
    println!(
        "  baseline           : {} ({}; P={}, R={})",
        baseline.run_id,
        baseline.model_version,
        baseline.evaluation.precision,
        baseline.evaluation.recall,
    );
    match run.evaluation.pinned_precision {
        Some(p) => println!(
            "  pinned@baseline P  : threshold={}, recall={}",
            p.threshold, p.recall
        ),
        None => println!("  pinned@baseline P  : no qualifying threshold"),
    }
    println!(
        "  test cost          : {}  (total run incl. iterations: {})",
        run.resources.cost,
        run.total_cost(),
    );
    println!("  appended to log    : {}", args.eval_results_path.display());
}

fn print_accepted(args: &Args, baseline: &RunRecord, report: &skeet_refine::train::TrainingReport) {
    println!();
    println!(
        "  ACCEPTED: candidate clears baseline precision={} with recall ≥ {} - {}",
        baseline.evaluation.precision,
        baseline.evaluation.recall,
        skeet_refine::train::gate::GATE_RECALL_TOLERANCE,
    );
    println!(
        "  saved decision_threshold : {}",
        report.candidate_model.decision_threshold
    );
    println!("  written model      : {}", args.model_output.display());
}

fn print_rejected(args: &Args, baseline: &RunRecord) {
    println!();
    println!(
        "  REJECTED: candidate did not match baseline precision={} at recall ≥ {} - {}",
        baseline.evaluation.precision,
        baseline.evaluation.recall,
        skeet_refine::train::gate::GATE_RECALL_TOLERANCE,
    );
    println!(
        "  refine.toml at {} left untouched",
        args.model_output.display()
    );
}
