use std::path::PathBuf;

use clap::Parser;
use skeet_scorer::model::load_model;
use skeet_scorer::scoring::{build_agent, create_client, score_image};
use skeet_store::StoreArgs;
use tracing::{error, info};

#[derive(Parser)]
#[command(
    name = "live-score",
    about = "Continuously score new unscored images in the store"
)]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to model.toml
    #[arg(long, default_value = "skeet-scorer/model.toml")]
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

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

        for image_id in &unscored_ids {
            let stored = match store.get_by_id(image_id).await? {
                Some(s) => s,
                None => {
                    error!(image_id = %image_id, "image not found in store");
                    continue;
                }
            };

            match score_image(&agent, &stored.image).await {
                Ok(score) => {
                    store
                        .upsert_score(image_id, &score, &model_version)
                        .await?;
                    info!(image_id = %image_id, %score, "scored");
                }
                Err(e) => {
                    error!(image_id = %image_id, error = %e, "failed to score");
                }
            }
        }
    }
}
