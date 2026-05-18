use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use clap::Parser;
use shared::ModelVersion;
use skeet_refine::batch::Batch;
use skeet_refine::llm_metrics::LlmMetrics;
use skeet_refine::metrics::LiveRefineMetrics;
use skeet_refine::model::{Label, RefineModels};
use skeet_refine::polling::PollingBatchSource;
use skeet_refine::refining::{build_agent, create_client, refine_image, RefineAgent};
use skeet_refine::tick::{RunningTotals, TickAccumulator};
use skeet_store::{StoreArgs, StoreMetrics};
use tracing::info;

#[derive(Parser)]
#[command(
    name = "live-refine",
    about = "Continuously refine new unscored images in the store"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to refine.toml
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// OpenAI API key
    #[arg(long, env = "BOBBY_OPENAI_API_KEY")]
    openai_api_key: String,

    /// Polling interval in seconds
    #[arg(long, default_value_t = 60)]
    interval_secs: u64,

    /// Maximum concurrent OpenAI requests
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

/// Static configuration for scoring images — shared across all ticks.
struct ScoringContext<'a> {
    agent: &'a RefineAgent,
    model_name: &'a str,
    model_version: &'a ModelVersion,
    concurrency: usize,
}

async fn dispatch(
    batch: &mut Batch,
    ctx: &ScoringContext<'_>,
    acc: &mut TickAccumulator,
    llm_metrics: &LlmMetrics,
) {
    let agent = ctx.agent;
    let model_name = ctx.model_name;
    let outcomes = batch
        .score_with(ctx.concurrency, move |img| async move {
            let start = Instant::now();
            match refine_image(agent, &img).await {
                Ok((score, usage, duration)) => {
                    llm_metrics.record_success(&usage, duration, model_name);
                    Ok(score)
                }
                Err(e) => {
                    llm_metrics.record_error(start.elapsed(), e.as_label(), model_name);
                    Err(e)
                }
            }
        })
        .await;
    acc.record_outcomes(outcomes, ctx.model_version);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file(
        "skeet_refine=info,shared=info,skeet_store=info,lance_io=warn,object_store=warn",
        "live-refine.log",
    );
    info!(git_hash = env!("BUILD_GIT_HASH"), "live-refine starting");

    let args = Args::parse();
    let models = RefineModels::load(&args.model_path)?;
    let model = models
        .by_label(&Label::production())
        .ok_or("no production label in refine.toml")?;
    let model_version = model.version();
    info!(model_name = %model.model_name, %model_version, "loaded model");

    let store = Arc::new(args.store.open_store("live_refine").await?);
    let client = create_client(&args.openai_api_key);
    let agent = build_agent(&client, model.model_name.as_str(), model.prompt.as_str());

    let ctx = ScoringContext {
        agent: &agent,
        model_name: model.model_name.as_str(),
        model_version: &model_version,
        concurrency: args.concurrency,
    };

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(args.interval_secs));
    let mut source = PollingBatchSource::new(store.clone());

    let mut metrics = LiveRefineMetrics::new(&opentelemetry::global::meter("skeet_live_refine"));
    let llm_metrics = LlmMetrics::new(&opentelemetry::global::meter("gen_ai"));
    let store_metrics = StoreMetrics::new(opentelemetry::global::meter("lance"));
    let mut totals = RunningTotals::new();

    loop {
        interval.tick().await;

        let mut candidates = source.fetch().await?;
        if candidates.is_empty() {
            continue;
        }

        let unscored_count = candidates.len() as u64;
        let mut acc = TickAccumulator::new();
        dispatch(&mut candidates, &ctx, &mut acc, &llm_metrics).await;

        totals.absorb_tick(unscored_count, &acc);

        let tick_scores = if acc.pending_scores.is_empty() {
            vec![]
        } else {
            let scores = acc.scores();
            store.batch_upsert_scores(&acc.pending_scores).await?;
            info!(
                scored = acc.pending_scores.len(),
                remaining = acc.remaining(unscored_count),
                "batch-saved scores"
            );
            scores
        };
        source.commit(candidates);

        metrics.emit(totals.unscored, totals.scored, &totals.errors, &tick_scores);

        if let Ok(versions) = store.table_versions().await {
            store_metrics.record_table_versions(&versions);
        }
    }
}
