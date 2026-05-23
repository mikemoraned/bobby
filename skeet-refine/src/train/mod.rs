pub mod budget;
pub mod gate;
pub mod refinement;
pub mod scoring;
pub mod setup;

use chrono::{DateTime, Utc};
use eval::{
    Evaluation, EvalSplit, F1, ModelPrices, PricingError, Purpose, Resources, RunId, RunRecord,
    SplitId, Usd, confusion_at, pin_at_precision, roc_auc_score,
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
    #[error("image id {0} is no longer present in the store appraisals")]
    AppraisalMissing(String),
    #[error("baseline run records zero scored images — cannot derive per-image cost")]
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
    pub split_id: SplitId,
    pub baseline: &'a RunRecord,
    pub prices: &'a ModelPrices,
    pub openai_api_key: &'a str,
    pub max_iterations: u32,
    pub budget: Usd,
    pub model: String,
    pub concurrency: usize,
    pub seed: u64,
    pub purpose: Purpose,
    pub run_at: DateTime<Utc>,
}

pub struct TrainingReport {
    pub run: RunRecord,
    pub outcome: GateOutcome,
    pub candidate_model: RefineModel,
    pub per_iter_size: usize,
}

impl<'a> TrainingInputs<'a> {
    pub async fn train(self) -> Result<TrainingReport, TrainError> {
        let baseline_image_count = self.baseline.evaluation.confusion.total();
        if baseline_image_count == 0 {
            return Err(TrainError::EmptyBaseline);
        }
        let per_image_cost = self.prices.cost_for(
            &self.model,
            self.baseline.resources.input_tokens / baseline_image_count,
            self.baseline.resources.output_tokens / baseline_image_count,
        )?;

        let per_iter_size = per_iteration_sample_size(
            per_image_cost,
            self.split.test.len(),
            self.budget,
            self.max_iterations,
        )?;
        info!(
            per_iter_size,
            budget = %self.budget,
            per_image_cost = %per_image_cost,
            max_iterations = self.max_iterations,
            "derived per-iteration sample size from current model cost estimate"
        );

        let band_by_id = load_band_index(self.store).await?;
        let train_items = label_train_items(&self.split.train, &band_by_id)?;
        info!(count = train_items.len(), "labelled train pool");

        let client = create_client(self.openai_api_key);

        let mut current_prompt = SEED_PROMPT.to_string();
        let mut best_prompt = current_prompt.clone();
        let mut best_train_f1: Option<F1> = None;
        let mut training_input = 0_u64;
        let mut training_output = 0_u64;

        for iteration in 1..=self.max_iterations {
            let iter_seed = self.seed.wrapping_add(u64::from(iteration));
            let sampled_ids: Vec<ImageId> =
                eval::stratified_sample(&train_items, per_iter_size, iter_seed);
            let sampled = load_labelled_images(self.store, &band_by_id, &sampled_ids).await?;
            info!(iteration, sample = sampled.len(), "starting iteration");

            let agent = build_agent(&client, &self.model, &current_prompt);
            let agent_ref = &agent;
            let scored = score_concurrent(&sampled, self.concurrency, |image| async move {
                refine_image(agent_ref, &image).await
            })
            .await?;
            let (in_t, out_t) = token_totals(&scored);
            training_input += in_t;
            training_output += out_t;

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

            if iteration < self.max_iterations {
                match refine_prompt(&client, &self.model, &current_prompt, &sampled, &scored).await
                {
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

        info!("training loop complete; running final test-set evaluation");

        let test_images = load_labelled_images(self.store, &band_by_id, &self.split.test).await?;
        let test_agent = build_agent(&client, &self.model, &best_prompt);
        let test_agent_ref = &test_agent;
        let test_scored =
            score_concurrent(&test_images, self.concurrency, |image| async move {
                refine_image(test_agent_ref, &image).await
            })
            .await?;
        let (test_in, test_out) = token_totals(&test_scored);

        let test_labelled = labelled_scores(&test_images, &test_scored);
        let test_matrix = confusion_at(&test_labelled, training_loop_threshold());
        let test_precision = test_matrix
            .precision()
            .ok_or(TrainError::NoPositivePredictions)?;
        let test_recall = test_matrix.recall().ok_or(TrainError::NoPositives)?;
        let test_f1 = test_matrix.f1().expect("precision and recall both defined");
        let test_roc_auc = roc_auc_score(&test_labelled);

        let test_cost = self.prices.cost_for(&self.model, test_in, test_out)?;
        let training_cost = self
            .prices
            .cost_for(&self.model, training_input, training_output)?;

        let pinned_at_baseline_precision =
            pin_at_precision(&test_labelled, self.baseline.evaluation.precision);
        let outcome = evaluate_gate(pinned_at_baseline_precision, self.baseline.evaluation.recall);

        let candidate_threshold = pinned_at_baseline_precision
            .map(|p| p.threshold)
            .unwrap_or_else(training_loop_threshold);
        let candidate_model =
            build_candidate_model(&self.model, &best_prompt, candidate_threshold);

        let run = RunRecord {
            run_id: RunId::from_run_at(self.run_at),
            run_at: self.run_at,
            model_version: candidate_model.version(),
            split_id: self.split_id,
            purpose: self.purpose,
            evaluation: Evaluation {
                precision: test_precision,
                recall: test_recall,
                f1: test_f1,
                roc_auc: test_roc_auc,
                pinned_precision: pinned_at_baseline_precision,
                confusion: test_matrix,
            },
            resources: Resources {
                input_tokens: test_in,
                output_tokens: test_out,
                cost: test_cost,
            },
            training: Some(Resources {
                input_tokens: training_input,
                output_tokens: training_output,
                cost: training_cost,
            }),
        };

        Ok(TrainingReport {
            run,
            outcome,
            candidate_model,
            per_iter_size,
        })
    }
}
