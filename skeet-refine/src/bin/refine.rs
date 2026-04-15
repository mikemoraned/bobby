use std::path::PathBuf;

use clap::Parser;
use skeet_refine::model::load_model;
use skeet_refine::refining::{build_agent, create_client, refine_image};
use skeet_store::StoreArgs;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "refine", about = "Score all images in the store using refine.toml")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to refine.toml
    #[arg(long, default_value = "config/refine.toml")]
    model_path: PathBuf,

    /// OpenAI API key
    #[arg(long, env = "BOBBY_OPENAI_API_KEY")]
    openai_api_key: String,
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

    let images = store.list_all().await?;
    let total = images.len();
    info!(total, "refining all images");

    for (i, stored) in images.iter().enumerate() {
        let image_id = &stored.summary.image_id;
        match refine_image(&agent, &stored.image).await {
            Ok(score) => {
                store
                    .upsert_score(image_id, &score, &model_version)
                    .await?;
                info!(
                    progress = format!("{}/{}", i + 1, total),
                    image_id = %image_id,
                    %score,
                    "refined"
                );
            }
            Err(e) => {
                error!(image_id = %image_id, error = %e, "failed to refine");
            }
        }
    }

    info!(total, "refining complete");
    Ok(())
}
