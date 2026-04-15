use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use futures::stream::{self, StreamExt};
use image::DynamicImage;
use shared::{ModelVersion, Score};
use skeet_refine::model::load_model;
use skeet_refine::refining::{build_agent, create_client, refine_image, RefineAgent};
use skeet_store::{ImageId, StoreArgs};
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file(
        "skeet_refine=info,shared=info,skeet_store=info,lance_io=warn,object_store=warn",
        "live-refine.log",
        shared::tracing::TokioConsoleSupport::Disabled,
    );

    let args = Args::parse();
    let model = load_model(&args.model_path)?;
    let model_version = model.version();
    info!(model_name = %model.model_name, %model_version, "loaded model");

    let store = args.store.open_store().await?;
    let client = create_client(&args.openai_api_key);
    let agent = build_agent(&client, model.model_name.as_str(), model.prompt.as_str());

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(args.interval_secs));

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

        // Fetch images, then dispatch OpenAI calls in parallel batches
        let mut pending_scores = Vec::new();
        let mut batch_ids = Vec::new();
        let mut batch_images = Vec::new();

        for image_id in &unscored_ids {
            if started.elapsed() >= budget {
                break;
            }

            match store.get_by_id(image_id).await? {
                Some(stored) => {
                    batch_ids.push(image_id.clone());
                    batch_images.push(stored.image);
                }
                None => {
                    error!(image_id = %image_id, "image not found in store");
                }
            }

            // When we have a full batch (or budget is about to expire), dispatch in parallel
            if batch_ids.len() >= args.concurrency || started.elapsed() >= budget {
                dispatch_batch(&agent, &mut batch_ids, &mut batch_images, &model_version, args.concurrency, &mut pending_scores).await;
            }
        }

        // Dispatch any remaining images in the last partial batch
        if !batch_ids.is_empty() {
            dispatch_batch(&agent, &mut batch_ids, &mut batch_images, &model_version, args.concurrency, &mut pending_scores).await;
        }

        // Batch-save all scores in one write
        if !pending_scores.is_empty() {
            let scored = pending_scores.len();
            store.batch_upsert_scores(&pending_scores).await?;
            let remaining = unscored_ids.len() - scored;
            info!(scored, remaining, "batch-saved scores");
        }
    }
}

async fn dispatch_batch(
    agent: &RefineAgent,
    batch_ids: &mut Vec<ImageId>,
    batch_images: &mut Vec<DynamicImage>,
    model_version: &ModelVersion,
    concurrency: usize,
    pending_scores: &mut Vec<(ImageId, Score, ModelVersion)>,
) {
    let results: Vec<_> = stream::iter(batch_images.iter())
        .map(|image| refine_image(agent, image))
        .buffer_unordered(concurrency)
        .collect()
        .await;

    for (id, result) in batch_ids.drain(..).zip(results) {
        match result {
            Ok(score) => {
                info!(image_id = %id, %score, "refined");
                pending_scores.push((id, score, model_version.clone()));
            }
            Err(e) => {
                error!(image_id = %id, error = %e, "failed to refine");
            }
        }
    }
    batch_images.clear();
}
