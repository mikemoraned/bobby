use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use futures::stream::{self, StreamExt};
use image::DynamicImage;
use shared::{ModelVersion, Score};
use skeet_refine::metrics::LiveRefineMetrics;
use skeet_refine::model::load_model;
use skeet_refine::refining::{build_agent, create_client, refine_image, RefineAgent};
use skeet_store::{ImageId, StoreArgs, StoreMetrics};
use tracing::{error, info};

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
    model_version: &'a ModelVersion,
    concurrency: usize,
}

/// A staging buffer of images to be scored together in one parallel dispatch.
struct Batch {
    ids: Vec<ImageId>,
    images: Vec<DynamicImage>,
}

impl Batch {
    fn new() -> Self {
        Self {
            ids: Vec::new(),
            images: Vec::new(),
        }
    }

    fn push(&mut self, id: ImageId, image: DynamicImage) {
        self.ids.push(id);
        self.images.push(image);
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    async fn dispatch(&mut self, ctx: &ScoringContext<'_>, acc: &mut TickAccumulator) {
        let results: Vec<_> = stream::iter(self.images.iter())
            .map(|image| refine_image(ctx.agent, image))
            .buffer_unordered(ctx.concurrency)
            .collect()
            .await;

        for (id, result) in self.ids.drain(..).zip(results) {
            match result {
                Ok(score) => {
                    info!(image_id = %id, %score, "refined");
                    acc.pending_scores.push((id, score, ctx.model_version.clone()));
                }
                Err(e) => {
                    error!(image_id = %id, error = %e, "failed to refine");
                    *acc.errors.entry(e.as_label().to_string()).or_default() += 1;
                }
            }
        }
        self.images.clear();
    }
}

/// Running totals accumulated across all ticks, used to drive OTel metrics.
struct RunningTotals {
    unscored: u64,
    scored: u64,
    errors: HashMap<String, u64>,
}

impl RunningTotals {
    fn new() -> Self {
        Self {
            unscored: 0,
            scored: 0,
            errors: HashMap::new(),
        }
    }

    fn absorb_tick(&mut self, unscored_count: u64, acc: &TickAccumulator) {
        self.unscored += unscored_count;
        self.scored += acc.pending_scores.len() as u64;
        acc.merge_errors_into(&mut self.errors);
    }
}

/// Mutable state accumulated within a single tick.
struct TickAccumulator {
    pending_scores: Vec<(ImageId, Score, ModelVersion)>,
    errors: HashMap<String, u64>,
}

impl TickAccumulator {
    fn new() -> Self {
        Self {
            pending_scores: Vec::new(),
            errors: HashMap::new(),
        }
    }

    fn merge_errors_into(&self, totals: &mut HashMap<String, u64>) {
        for (reason, count) in &self.errors {
            *totals.entry(reason.clone()).or_default() += count;
        }
    }

    /// Extract scores as f64 observations for the histogram.
    fn scores(&self) -> Vec<f64> {
        self.pending_scores
            .iter()
            .map(|(_, s, _)| f64::from(*s))
            .collect()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file(
        "skeet_refine=info,shared=info,skeet_store=info,lance_io=warn,object_store=warn",
        "live-refine.log",
    );
    info!(git_hash = env!("BUILD_GIT_HASH"), "live-refine starting");

    let args = Args::parse();
    let model = load_model(&args.model_path)?;
    let model_version = model.version();
    info!(model_name = %model.model_name, %model_version, "loaded model");

    let store = args.store.open_store("live_refine").await?;
    let client = create_client(&args.openai_api_key);
    let agent = build_agent(&client, model.model_name.as_str(), model.prompt.as_str());

    let ctx = ScoringContext {
        agent: &agent,
        model_version: &model_version,
        concurrency: args.concurrency,
    };

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(args.interval_secs));

    let mut metrics = LiveRefineMetrics::new();
    let store_metrics = StoreMetrics::new(opentelemetry::global::meter("lance"));
    let mut totals = RunningTotals::new();

    loop {
        interval.tick().await;

        let unscored_ids = store
            .list_unscored_image_ids_for_version(&model_version)
            .await?;
        if unscored_ids.is_empty() {
            info!("no unscored images");
            continue;
        }

        info!(count = unscored_ids.len(), "found unscored images");

        let budget = std::time::Duration::from_secs(args.interval_secs);
        let started = Instant::now();

        let originals = store.get_originals_by_ids(&unscored_ids).await?;
        let mut originals_map: HashMap<ImageId, DynamicImage> = originals
            .into_iter()
            .map(|s| (s.summary.image_id, s.image))
            .collect();

        let mut acc = TickAccumulator::new();
        let mut batch = Batch::new();

        for image_id in &unscored_ids {
            if started.elapsed() >= budget {
                break;
            }

            match originals_map.remove(image_id) {
                Some(image) => batch.push(image_id.clone(), image),
                None => error!(image_id = %image_id, "image not found in store"),
            }

            if batch.len() >= ctx.concurrency || started.elapsed() >= budget {
                batch.dispatch(&ctx, &mut acc).await;
            }
        }

        if !batch.is_empty() {
            batch.dispatch(&ctx, &mut acc).await;
        }

        totals.absorb_tick(unscored_ids.len() as u64, &acc);

        let scored = acc.pending_scores.len();
        let tick_scores = if scored > 0 {
            let scores = acc.scores();
            store.batch_upsert_scores(&acc.pending_scores).await?;
            let remaining = unscored_ids.len() - scored;
            info!(scored, remaining, "batch-saved scores");
            scores
        } else {
            vec![]
        };

        metrics.emit(totals.unscored, totals.scored, &totals.errors, &tick_scores);

        if let Ok(versions) = store.table_versions().await {
            store_metrics.record_table_versions(&versions);
        }
    }
}
