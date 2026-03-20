#![warn(clippy::all, clippy::nursery)]

use std::path::PathBuf;

use clap::Parser;
use skeet_store::{ImageId, StoreArgs};
use tracing::info;

#[derive(Parser)]
#[command(about = "Export an image from the store to a file")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    #[arg(long)]
    image_id: ImageId,

    #[arg(long)]
    output: PathBuf,

    /// Export the annotated image instead of the original
    #[arg(long, default_value_t = false)]
    annotated: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let store = args.store.open_store().await?;
    let stored = store
        .get_by_id(&args.image_id)
        .await?
        .ok_or_else(|| format!("no image found with id {}", args.image_id))?;

    let image = if args.annotated {
        &stored.annotated_image
    } else {
        &stored.image
    };

    image.save(&args.output)?;

    info!(output = %args.output.display(), "saved image");
    Ok(())
}
