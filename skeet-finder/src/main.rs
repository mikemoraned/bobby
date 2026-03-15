#![warn(clippy::all, clippy::nursery)]

mod classify_and_store;
mod firehose;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use face_detection::{ArchetypeConfig, FaceDetector, Rejection};
use indicatif::{ProgressBar, ProgressStyle};
use skeet_store::SkeetStore;
use tracing::{info, warn};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store_path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "skeet_finder=info".parse().expect("valid filter")),
        )
        .init();

    let args = Args::parse();

    let store = SkeetStore::open(&args.store_path).await?;
    store.validate().await?;
    info!("storage validation passed");

    let http = reqwest::Client::new();
    let detector = FaceDetector::from_bundled_weights();
    let text_detector = text_detection::TextDetector::from_bundled_models();

    let archetype_config = ArchetypeConfig::from_file(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/archetype.toml"),
    )?;
    let config_version = archetype_config.version();

    info!(config_version = %config_version, "face detection model loaded");

    let receiver = firehose::connect().await?;

    let spinner = ProgressBar::new_spinner();
    #[allow(clippy::literal_string_with_formatting_args)]
    let style = ProgressStyle::with_template("{elapsed_precise} {spinner} {msg}")
        .expect("valid template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏-");
    spinner.set_style(style);
    spinner.enable_steady_tick(Duration::from_millis(100));
    spinner.set_message("connected, listening for posts...");

    let mut post_count: u64 = 0;
    let mut image_post_count: u64 = 0;
    let mut saved_count: u64 = 0;
    let mut rejection_counts: HashMap<Rejection, u64> = HashMap::new();

    while let Ok(event) = receiver.recv_async().await {
        post_count += 1;

        let skeet_images = firehose::extract_skeet_images(&event, &http).await;
        if skeet_images.is_empty() {
            if post_count.is_multiple_of(500) {
                update_spinner(
                    &spinner,
                    post_count,
                    image_post_count,
                    saved_count,
                    &rejection_counts,
                );
            }
            continue;
        }

        image_post_count += 1;

        for skeet_image in skeet_images {
            match classify_and_store::classify_image(
                skeet_image,
                &detector,
                &text_detector,
                &archetype_config,
                &config_version,
            ) {
                Ok(record) => {
                    classify_and_store::save(&store, &record, &mut saved_count).await;
                }
                Err(reasons) => {
                    for reason in &reasons {
                        *rejection_counts.entry(*reason).or_default() += 1;
                    }
                }
            }
            update_spinner(
                &spinner,
                post_count,
                image_post_count,
                saved_count,
                &rejection_counts,
            );
        }
    }

    spinner.finish_with_message("jetstream connection closed");
    warn!("jetstream connection closed");
    Ok(())
}

fn update_spinner(
    spinner: &ProgressBar,
    posts: u64,
    images: u64,
    saved: u64,
    rejections: &HashMap<Rejection, u64>,
) {
    let hit_rate = if images > 0 {
        (saved as f64 / images as f64) * 100.0
    } else {
        0.0
    };

    let mut msg = format!(
        "skeets: {posts} | images: {images} | saved: {saved} ({hit_rate:.1}%)"
    );

    if !rejections.is_empty() {
        let total_rejections: u64 = rejections.values().sum();
        let mut sorted: Vec<_> = rejections.iter().collect();
        sorted.sort_by_key(|(r, _)| r.to_string());

        write!(msg, " | rejected: {total_rejections} (").expect("write to String");
        for (i, (reason, count)) in sorted.iter().enumerate() {
            let pct = (**count as f64 / total_rejections as f64) * 100.0;
            if i > 0 {
                write!(msg, ", ").expect("write to String");
            }
            write!(msg, "{reason}: {count} [{pct:.0}%]").expect("write to String");
        }
        write!(msg, ")").expect("write to String");
    }

    spinner.set_message(msg);
}
