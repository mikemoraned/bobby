use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use skeet_refine::model::load_model;
use skeet_refine::refining::{build_agent, create_client, refine_image};
use skeet_store::StoreArgs;
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = shared::tracing::init_with_file_and_stderr(
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
        let mut scored = 0u64;

        for image_id in &unscored_ids {
            let stored = match store.get_by_id(image_id).await? {
                Some(s) => s,
                None => {
                    error!(image_id = %image_id, "image not found in store");
                    continue;
                }
            };

            match refine_image(&agent, &stored.image).await {
                Ok(score) => {
                    store
                        .upsert_score(image_id, &score, &model_version)
                        .await?;
                    scored += 1;
                    info!(image_id = %image_id, %score, "refined");
                }
                Err(e) => {
                    error!(image_id = %image_id, error = %e, "failed to refine");
                }
            }

            if started.elapsed() >= budget {
                let remaining = unscored_ids.len() as u64 - scored;
                info!(scored, remaining, "scoring budget elapsed, re-checking for newer images");
                break;
            }
        }
    }
}
