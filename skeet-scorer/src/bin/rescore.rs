use std::path::PathBuf;

use clap::Parser;
use skeet_scorer::model::load_model;
use skeet_scorer::scoring::{build_agent, create_client, score_image};
use skeet_store::StoreArgs;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "rescore", about = "Score all images in the store using model.toml")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to model.toml
    #[arg(long, default_value = "skeet-scorer/model.toml")]
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
    info!(model_name = %model.model_name, "loaded model");

    let store = args.store.open_store().await?;
    let client = create_client(&args.openai_api_key);
    let agent = build_agent(&client, model.model_name.as_str(), model.prompt.as_str());

    let images = store.list_all().await?;
    let total = images.len();
    info!(total, "scoring all images");

    for (i, stored) in images.iter().enumerate() {
        let image_id = &stored.summary.image_id;
        match score_image(&agent, &stored.image).await {
            Ok(score) => {
                store.upsert_score(image_id, score).await?;
                info!(
                    progress = format!("{}/{}", i + 1, total),
                    image_id = %image_id,
                    score,
                    "scored"
                );
            }
            Err(e) => {
                error!(image_id = %image_id, error = %e, "failed to score");
            }
        }
    }

    info!(total, "rescoring complete");
    Ok(())
}
