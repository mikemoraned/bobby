use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use eval::{
    EvalResults, EvalSplit, F1, LabelledScore, ModelPrices, PinnedPrecision, Recall, Threshold,
    confusion_at, pin_at_precision, roc_auc_score, stratified_sample,
};
use futures::stream::{self, StreamExt};
use rig::agent::AgentBuilder;
use rig::client::CompletionClient;
use rig::completion::request::Prompt;
use shared::{Band, Score};
use skeet_refine::loader::{LabelledImage, load_band_index, load_labelled_images};
use skeet_refine::model::{Label, ModelName, ModelProvider, RefineModel, RefineModels, RefinePrompt};
use skeet_refine::refining::{RefineAgent, SEED_PROMPT, build_agent, create_client, refine_image};
use skeet_store::{ImageId, StoreArgs};
use tracing::{error, info, warn};

/// Threshold used during the training loop to score in-loop F1 — the prompt
/// with the best F1 at this threshold becomes the candidate. The deployed
/// threshold is decided separately after the loop by pinning at the baseline's
/// precision floor.
const TRAINING_LOOP_THRESHOLD_F64: f64 = 0.5;

/// Maximum tolerated drop in recall (absolute) at the baseline's precision
/// floor before the candidate is rejected.
const GATE_RECALL_TOLERANCE: f64 = 0.01;

fn training_loop_threshold() -> Threshold {
    Threshold::new(TRAINING_LOOP_THRESHOLD_F64).expect("0.5 is in [0, 1]")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateOutcome {
    Accepted,
    Rejected,
}

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
    #[arg(long, default_value_t = 5.0)]
    budget_usd: f64,

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

#[derive(Debug, thiserror::Error)]
enum TrainError {
    #[error(
        "split content hash {split_hash} does not match the baseline's recorded hash {baseline_hash} — re-run capture-appraisals + refine-eval to refresh the baseline before re-training"
    )]
    SplitHashDrift {
        split_hash: String,
        baseline_hash: String,
    },
    #[error("invalid image id in split: {0}")]
    InvalidImageId(String),
    #[error("image id {0} is no longer present in the store appraisals")]
    AppraisalMissing(String),
    #[error("baseline records zero scored images — cannot derive per-image cost")]
    EmptyBaseline,
    #[error("derived per-iteration sample size is zero — increase --budget-usd")]
    BudgetTooSmall,
    #[error("no positive labels in test set")]
    NoPositives,
    #[error("no positive predictions in test set at threshold 0.5")]
    NoPositivePredictions,
    #[error("prompt refinement call failed at iteration {iteration}: {message}")]
    PromptRefinementFailed { iteration: u32, message: String },
}

#[derive(Debug, Clone, Copy)]
struct ScoredCall {
    score: Score,
    input_tokens: u64,
    output_tokens: u64,
}

async fn score_concurrent(
    agent: &RefineAgent,
    images: &[LabelledImage],
    concurrency: usize,
) -> Result<Vec<ScoredCall>, Box<dyn std::error::Error>> {
    let total = images.len();
    let scored: Vec<ScoredCall> = stream::iter(images.iter())
        .map(|labelled| async move {
            refine_image(agent, &labelled.image)
                .await
                .map(|(score, usage, _d)| ScoredCall {
                    score,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                })
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

fn labelled_scores(images: &[LabelledImage], scored: &[ScoredCall]) -> Vec<LabelledScore> {
    images
        .iter()
        .zip(scored.iter())
        .map(|(img, call)| LabelledScore {
            score: call.score,
            is_positive: img.is_positive(),
        })
        .collect()
}

fn token_totals(scored: &[ScoredCall]) -> (u64, u64) {
    scored.iter().fold((0, 0), |(i, o), c| {
        (i + c.input_tokens, o + c.output_tokens)
    })
}

fn format_results_for_refinement(images: &[LabelledImage], scored: &[ScoredCall]) -> String {
    let mut s = String::from("Here are the scoring results for each example:\n\n");
    for (img, call) in images.iter().zip(scored.iter()) {
        let is_pos = img.is_positive();
        let score: f32 = call.score.into();
        let predicted_pos = score > 0.5;
        let status = if is_pos == predicted_pos { "CORRECT" } else { "WRONG" };
        let expected = if is_pos { "high (>0.5)" } else { "low (<=0.5)" };
        s.push_str(&format!(
            "- {}: score={:.2}, expected={}, status={}\n",
            img.id, score, expected, status
        ));
    }
    s
}

async fn refine_prompt(
    client: &rig::providers::openai::client::CompletionsClient,
    model_name: &str,
    current_prompt: &str,
    images: &[LabelledImage],
    scored: &[ScoredCall],
) -> Result<String, Box<dyn std::error::Error>> {
    let labelled = labelled_scores(images, scored);
    let matrix = confusion_at(&labelled, training_loop_threshold());
    let f1_pct = matrix
        .f1()
        .map(|v| format!("{:.0}%", f64::from(v) * 100.0))
        .unwrap_or_else(|| "undefined".to_string());
    let results_summary = format_results_for_refinement(images, scored);

    let refinement_request = format!(
        "You are helping improve a scoring prompt for an image classification system.\n\n\
         The current scoring prompt is:\n\
         ---\n{current_prompt}\n---\n\n\
         {results_summary}\n\
         Current train F1: {f1_pct}\n\n\
         The expected=high images are good selfies with landmarks. The expected=low images should get low scores.\n\n\
         Please provide an improved scoring prompt that would better distinguish between good and bad examples.\n\
         Respond with ONLY the new prompt text, nothing else. Do not include any preamble or explanation."
    );

    let refinement_model = client.completion_model(model_name);
    let agent = AgentBuilder::new(refinement_model).temperature(0.0).build();
    let response = agent.prompt(refinement_request).await?;
    Ok(response)
}

fn label_train_items(
    train: &[String],
    band_by_id: &std::collections::HashMap<ImageId, Band>,
) -> Result<Vec<(String, Band)>, TrainError> {
    train
        .iter()
        .map(|s| {
            let id = ImageId::from_str(s).map_err(|_| TrainError::InvalidImageId(s.clone()))?;
            let band = band_by_id
                .get(&id)
                .copied()
                .ok_or_else(|| TrainError::AppraisalMissing(s.clone()))?;
            Ok((s.clone(), band))
        })
        .collect()
}

/// Build the `RefineModel` that, on Accept, will be appended to the registry
/// and labelled `production`.
fn build_candidate_model(model_name: &str, prompt: &str, threshold: Threshold) -> RefineModel {
    RefineModel {
        model_provider: ModelProvider::openai(),
        model_name: ModelName::new(model_name),
        prompt: RefinePrompt::new(prompt),
        decision_threshold: threshold,
    }
}

/// Sample size per iteration. Reserves cost for one full test-set evaluation
/// at the baseline's per-image cost, then divides the residual budget evenly
/// across `max_iterations`.
fn per_iteration_sample_size(
    baseline: &EvalResults,
    test_count: usize,
    budget_usd: f64,
    max_iterations: u32,
) -> Result<usize, TrainError> {
    let baseline_images = baseline.tp + baseline.fp + baseline.tn + baseline.fn_;
    if baseline_images == 0 {
        return Err(TrainError::EmptyBaseline);
    }
    let per_image_cost = baseline.cost_usd / baseline_images as f64;
    let reserved_for_final_eval = per_image_cost * test_count as f64;
    let residual = (budget_usd - reserved_for_final_eval).max(0.0);
    let per_iter_budget = residual / max_iterations as f64;
    let size = (per_iter_budget / per_image_cost).floor() as usize;
    if size == 0 {
        return Err(TrainError::BudgetTooSmall);
    }
    Ok(size)
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
    let split_hash = split.content_hash();
    let baseline = EvalResults::load(&args.baseline_path)?;
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

    let per_iter_size = per_iteration_sample_size(
        &baseline,
        split.test.len(),
        args.budget_usd,
        args.max_iterations,
    )?;
    info!(
        per_iter_size,
        budget_usd = args.budget_usd,
        max_iterations = args.max_iterations,
        "derived per-iteration sample size from baseline cost"
    );

    let store = args.store.open_store("train").await?;
    let band_by_id = load_band_index(&store).await?;
    let train_items = label_train_items(&split.train, &band_by_id)?;
    info!(count = train_items.len(), "labelled train pool");

    let client = create_client(&args.openai_api_key);

    let mut current_prompt = SEED_PROMPT.to_string();
    let mut best_prompt = current_prompt.clone();
    let mut best_train_f1: Option<F1> = None;
    let mut total_input = 0_u64;
    let mut total_output = 0_u64;

    for iteration in 1..=args.max_iterations {
        let iter_seed = args.seed.wrapping_add(u64::from(iteration));
        let sampled_ids: Vec<String> =
            stratified_sample(&train_items, per_iter_size, iter_seed);
        let sampled = load_labelled_images(&store, &band_by_id, &sampled_ids).await?;
        info!(iteration, sample = sampled.len(), "starting iteration");

        let agent = build_agent(&client, &args.model, &current_prompt);
        let scored = score_concurrent(&agent, &sampled, args.concurrency).await?;
        let (in_t, out_t) = token_totals(&scored);
        total_input += in_t;
        total_output += out_t;

        let labelled = labelled_scores(&sampled, &scored);
        let matrix = confusion_at(&labelled, training_loop_threshold());
        let train_f1 = matrix.f1();
        info!(
            iteration,
            train_f1 = ?train_f1,
            tp = matrix.true_pos, fp = matrix.false_pos,
            tn = matrix.true_neg, fn_ = matrix.false_neg,
            "iteration scored"
        );

        if let Some(f1) = train_f1
            && best_train_f1.is_none_or(|best| f1 > best)
        {
            best_train_f1 = Some(f1);
            best_prompt = current_prompt.clone();
            info!(best_train_f1 = %f1, prompt = %current_prompt, "new best prompt");
        }

        if iteration < args.max_iterations {
            match refine_prompt(&client, &args.model, &current_prompt, &sampled, &scored).await {
                Ok(new_prompt) => {
                    info!(prompt = %new_prompt, "generated refined prompt");
                    current_prompt = new_prompt;
                }
                Err(e) => {
                    error!(iteration, error = %e, "prompt refinement call failed; aborting training");
                    return Err(TrainError::PromptRefinementFailed {
                        iteration,
                        message: e.to_string(),
                    }
                    .into());
                }
            }
        }
    }

    info!(
        best_train_f1 = ?best_train_f1,
        "training loop complete; running final test-set evaluation"
    );

    let test_images = load_labelled_images(&store, &band_by_id, &split.test).await?;
    let test_agent = build_agent(&client, &args.model, &best_prompt);
    let test_scored = score_concurrent(&test_agent, &test_images, args.concurrency).await?;
    let (test_in, test_out) = token_totals(&test_scored);
    total_input += test_in;
    total_output += test_out;

    let test_labelled = labelled_scores(&test_images, &test_scored);
    let test_matrix = confusion_at(&test_labelled, training_loop_threshold());
    let test_precision = test_matrix
        .precision()
        .ok_or(TrainError::NoPositivePredictions)?;
    let test_recall = test_matrix.recall().ok_or(TrainError::NoPositives)?;
    let test_f1 = test_matrix.f1().expect("precision and recall both defined");
    let test_roc_auc = roc_auc_score(&test_labelled);

    let prices = ModelPrices::embedded()?;
    let test_cost = prices.cost_for(&args.model, test_in, test_out)?;
    let total_cost = prices.cost_for(&args.model, total_input, total_output)?;

    let pinned_at_baseline_precision = pin_at_precision(&test_labelled, baseline.precision);
    let outcome = evaluate_gate(pinned_at_baseline_precision, baseline.recall);

    let candidate_threshold = pinned_at_baseline_precision
        .map(|p| p.threshold)
        .unwrap_or_else(training_loop_threshold);
    let candidate_model = build_candidate_model(&args.model, &best_prompt, candidate_threshold);
    let candidate_version = candidate_model.version().to_string();

    let results = EvalResults {
        split_config_path: args.split_path.display().to_string(),
        split_config_hash: split_hash,
        model_version: candidate_version.clone(),
        model_name: args.model.clone(),
        precision: test_precision,
        recall: test_recall,
        f1: test_f1,
        roc_auc: test_roc_auc,
        pinned_precision: pinned_at_baseline_precision,
        tp: test_matrix.true_pos,
        fp: test_matrix.false_pos,
        tn: test_matrix.true_neg,
        fn_: test_matrix.false_neg,
        input_tokens: test_in,
        output_tokens: test_out,
        cost_usd: test_cost,
    };
    results.save(&args.eval_output)?;

    println!();
    println!("=== Training results ===");
    println!("  model              : {} ({})", args.model, candidate_version);
    println!("  iterations         : {}", args.max_iterations);
    println!("  per-iter sample    : {per_iter_size}");
    if let Some(best) = best_train_f1 {
        println!("  best train F1      : {best}");
    } else {
        println!("  best train F1      : (undefined in every iteration)");
    }
    println!("  test precision     : {test_precision}");
    println!("  test recall        : {test_recall}");
    println!("  test F1            : {test_f1}");
    match test_roc_auc {
        Some(v) => println!("  test ROC-AUC       : {v}"),
        None => println!("  test ROC-AUC       : (undefined — only one class present)"),
    }
    println!(
        "  baseline precision : {} (recall {})",
        baseline.precision, baseline.recall
    );
    match pinned_at_baseline_precision {
        Some(p) => println!(
            "  pinned@baseline P  : threshold={}, recall={}",
            p.threshold, p.recall
        ),
        None => println!("  pinned@baseline P  : no qualifying threshold"),
    }
    println!(
        "  test cost          : ${test_cost:.4}  (total run incl. iterations: ${total_cost:.4})"
    );
    println!("  written eval       : {}", args.eval_output.display());

    match outcome {
        GateOutcome::Accepted => {
            models.insert(candidate_model.clone(), &[Label::production()]);
            models.save(&args.model_output)?;
            println!();
            println!(
                "  ACCEPTED: candidate clears baseline precision={} with recall ≥ {} - {GATE_RECALL_TOLERANCE}",
                baseline.precision, baseline.recall
            );
            println!(
                "  saved decision_threshold : {}",
                candidate_model.decision_threshold
            );
            println!("  written model      : {}", args.model_output.display());
            info!(
                path = %args.model_output.display(),
                decision_threshold = %candidate_model.decision_threshold,
                "saved new refine.toml"
            );
        }
        GateOutcome::Rejected => {
            println!();
            println!(
                "  REJECTED: candidate did not match baseline precision={} at recall ≥ {} - {GATE_RECALL_TOLERANCE}",
                baseline.precision, baseline.recall
            );
            println!(
                "  refine.toml at {} left untouched",
                args.model_output.display()
            );
            warn!("acceptance gate rejected candidate; refine.toml left untouched");
        }
    }

    if total_cost > args.budget_usd {
        let overshoot = total_cost - args.budget_usd;
        let pct = overshoot / args.budget_usd * 100.0;
        println!();
        println!(
            "  BUDGET OVERSHOOT   : total ${total_cost:.4} exceeds --budget-usd ${:.4} by ${overshoot:.4} ({pct:.1}%)",
            args.budget_usd
        );
        warn!(
            total_cost,
            budget_usd = args.budget_usd,
            overshoot,
            overshoot_pct = pct,
            "training run exceeded budget"
        );
    }

    Ok(())
}

/// Decide whether the candidate clears the baseline. At the baseline's
/// precision floor, the candidate's recall must be within
/// `GATE_RECALL_TOLERANCE` absolute of the baseline's recall.
fn evaluate_gate(pinned: Option<PinnedPrecision>, baseline_recall: Recall) -> GateOutcome {
    match pinned {
        Some(p) if f64::from(p.recall) >= f64::from(baseline_recall) - GATE_RECALL_TOLERANCE => {
            GateOutcome::Accepted
        }
        _ => GateOutcome::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_with(images: u64, cost: f64) -> EvalResults {
        EvalResults {
            split_config_path: "x".into(),
            split_config_hash: "x".into(),
            model_version: "x".into(),
            model_name: "x".into(),
            precision: eval::Precision::new(0.5).expect("valid"),
            recall: eval::Recall::new(0.5).expect("valid"),
            f1: F1::new(0.5).expect("valid"),
            roc_auc: None,
            pinned_precision: None,
            tp: images,
            fp: 0,
            tn: 0,
            fn_: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: cost,
        }
    }

    #[test]
    fn per_iteration_size_reserves_final_eval_cost() {
        // 143 baseline images at $0.30 → ~$0.0021/image.
        // Budget $5: reserve 143×0.0021 ≈ $0.30 for final eval; residual ≈ $4.70
        // across 10 iterations ≈ $0.47/iter → ~224 images/iter.
        let baseline = baseline_with(143, 0.3);
        let n = per_iteration_sample_size(&baseline, 143, 5.0, 10).expect("budget covers it");
        assert!(n > 0);
        let per_image: f64 = 0.3 / 143.0;
        let expected = ((5.0 - per_image * 143.0) / 10.0 / per_image).floor() as usize;
        assert_eq!(n, expected);
    }

    #[test]
    fn per_iteration_size_errors_on_zero_budget() {
        let baseline = baseline_with(143, 0.3);
        let err = per_iteration_sample_size(&baseline, 143, 0.0, 10);
        assert!(matches!(err, Err(TrainError::BudgetTooSmall)));
    }

    #[test]
    fn gate_accepts_when_recall_within_tolerance() {
        let baseline_recall = Recall::new(0.70).expect("valid");
        let pinned = PinnedPrecision {
            threshold: Threshold::new(0.5).expect("valid"),
            recall: Recall::new(f64::from(baseline_recall) - GATE_RECALL_TOLERANCE / 2.0)
                .expect("valid"),
        };
        assert_eq!(
            evaluate_gate(Some(pinned), baseline_recall),
            GateOutcome::Accepted
        );
    }

    #[test]
    fn gate_rejects_when_recall_drops_below_tolerance() {
        let baseline_recall = Recall::new(0.70).expect("valid");
        let pinned = PinnedPrecision {
            threshold: Threshold::new(0.5).expect("valid"),
            recall: Recall::new(f64::from(baseline_recall) - GATE_RECALL_TOLERANCE * 1.5)
                .expect("valid"),
        };
        assert_eq!(
            evaluate_gate(Some(pinned), baseline_recall),
            GateOutcome::Rejected
        );
    }

    #[test]
    fn gate_rejects_when_no_qualifying_threshold() {
        assert_eq!(
            evaluate_gate(None, Recall::new(0.5).expect("valid")),
            GateOutcome::Rejected
        );
    }

    #[test]
    fn inserting_candidate_makes_it_resolvable_by_production_label() {
        let candidate = build_candidate_model(
            "gpt-4o",
            "any prompt",
            Threshold::new(0.5).expect("valid"),
        );
        let mut models = RefineModels::new();
        models.insert(candidate.clone(), &[Label::production()]);

        let resolved = models
            .by_label(&Label::production())
            .expect("production label resolves");
        assert_eq!(resolved, &candidate);
    }
}
