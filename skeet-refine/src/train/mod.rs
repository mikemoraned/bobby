pub mod budget;
pub mod gate;
pub mod refinement;
pub mod scoring;
pub mod setup;

use eval::{
    EvalResults, EvalSplit, F1, ModelPrices, PricingError, Usd, confusion_at, pin_at_precision,
    roc_auc_score,
};
use shared::ImageId;
use shared::refine_model::RefineModel;
use skeet_store::SkeetStore;
use tracing::{error, info};

use crate::loader::{LoaderError, load_band_index, load_labelled_images};
use crate::refining::{RefineError, SEED_PROMPT, build_agent, create_client, refine_image};
use crate::train::budget::per_iteration_sample_size;
use crate::train::gate::{
    GateOutcome, build_candidate_model, evaluate_gate, training_loop_threshold,
};
use crate::train::refinement::refine_prompt;
use crate::train::scoring::{labelled_scores, score_concurrent, token_totals};
use crate::train::setup::label_train_items;

#[derive(Debug, thiserror::Error)]
pub enum TrainError {
    #[error(
        "split content hash {split_hash} does not match the baseline's recorded hash {baseline_hash} — re-run capture-appraisals + refine-eval to refresh the baseline before re-training"
    )]
    SplitHashDrift {
        split_hash: String,
        baseline_hash: String,
    },
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
    #[error("scoring failed: {0}")]
    Scoring(#[from] RefineError),
    #[error("loader error: {0}")]
    Loader(#[from] LoaderError),
    #[error("pricing error: {0}")]
    Pricing(#[from] PricingError),
}

pub struct TrainingInputs<'a> {
    pub store: &'a SkeetStore,
    pub split: &'a EvalSplit,
    pub split_path_str: String,
    pub split_hash: String,
    pub baseline: &'a EvalResults,
    pub prices: &'a ModelPrices,
    pub openai_api_key: &'a str,
    pub max_iterations: u32,
    pub budget: Usd,
    pub model: String,
    pub concurrency: usize,
    pub seed: u64,
}

pub struct TrainingReport {
    pub results: EvalResults,
    pub outcome: GateOutcome,
    pub candidate_model: RefineModel,
    pub total_cost: Usd,
    pub best_train_f1: Option<F1>,
    pub per_iter_size: usize,
}

pub async fn run_training(inputs: TrainingInputs<'_>) -> Result<TrainingReport, TrainError> {
    let TrainingInputs {
        store,
        split,
        split_path_str,
        split_hash,
        baseline,
        prices,
        openai_api_key,
        max_iterations,
        budget,
        model,
        concurrency,
        seed,
    } = inputs;

    let baseline_image_count = baseline.tp + baseline.fp + baseline.tn + baseline.fn_;
    if baseline_image_count == 0 {
        return Err(TrainError::EmptyBaseline);
    }
    let per_image_cost = prices.cost_for(
        &model,
        baseline.input_tokens / baseline_image_count,
        baseline.output_tokens / baseline_image_count,
    )?;

    let per_iter_size =
        per_iteration_sample_size(per_image_cost, split.test.len(), budget, max_iterations)?;
    info!(
        per_iter_size,
        budget = %budget,
        per_image_cost = %per_image_cost,
        max_iterations,
        "derived per-iteration sample size from current model cost estimate"
    );

    let band_by_id = load_band_index(store).await?;
    let train_items = label_train_items(&split.train, &band_by_id)?;
    info!(count = train_items.len(), "labelled train pool");

    let client = create_client(openai_api_key);

    let mut current_prompt = SEED_PROMPT.to_string();
    let mut best_prompt = current_prompt.clone();
    let mut best_train_f1: Option<F1> = None;
    let mut total_input = 0_u64;
    let mut total_output = 0_u64;

    for iteration in 1..=max_iterations {
        let iter_seed = seed.wrapping_add(u64::from(iteration));
        let sampled_ids: Vec<ImageId> =
            eval::stratified_sample(&train_items, per_iter_size, iter_seed);
        let sampled = load_labelled_images(store, &band_by_id, &sampled_ids).await?;
        info!(iteration, sample = sampled.len(), "starting iteration");

        let agent = build_agent(&client, &model, &current_prompt);
        let agent_ref = &agent;
        let scored = score_concurrent(&sampled, concurrency, |image| async move {
            refine_image(agent_ref, &image).await
        })
        .await?;
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

        if iteration < max_iterations {
            match refine_prompt(&client, &model, &current_prompt, &sampled, &scored).await {
                Ok(new_prompt) => {
                    info!(prompt = %new_prompt, "generated refined prompt");
                    current_prompt = new_prompt;
                }
                Err(e) => {
                    error!(iteration, error = %e, "prompt refinement call failed; aborting training");
                    return Err(TrainError::PromptRefinementFailed {
                        iteration,
                        message: e.to_string(),
                    });
                }
            }
        }
    }

    info!(
        best_train_f1 = ?best_train_f1,
        "training loop complete; running final test-set evaluation"
    );

    let test_images = load_labelled_images(store, &band_by_id, &split.test).await?;
    let test_agent = build_agent(&client, &model, &best_prompt);
    let test_agent_ref = &test_agent;
    let test_scored = score_concurrent(&test_images, concurrency, |image| async move {
        refine_image(test_agent_ref, &image).await
    })
    .await?;
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

    let test_cost = prices.cost_for(&model, test_in, test_out)?;
    let total_cost = prices.cost_for(&model, total_input, total_output)?;

    let pinned_at_baseline_precision = pin_at_precision(&test_labelled, baseline.precision);
    let outcome = evaluate_gate(pinned_at_baseline_precision, baseline.recall);

    let candidate_threshold = pinned_at_baseline_precision
        .map(|p| p.threshold)
        .unwrap_or_else(training_loop_threshold);
    let candidate_model = build_candidate_model(&model, &best_prompt, candidate_threshold);
    let candidate_version = candidate_model.version().to_string();

    let results = EvalResults {
        split_config_path: split_path_str,
        split_config_hash: split_hash,
        model_version: candidate_version,
        model_name: model,
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
        cost: test_cost,
    };

    Ok(TrainingReport {
        results,
        outcome,
        candidate_model,
        total_cost,
        best_train_f1,
        per_iter_size,
    })
}
