#![warn(clippy::all, clippy::nursery)]

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Parser;
use eval::{ConfusionMatrix, F1, Precision, Recall};
use face_detection::FaceDetector;
use serde::Serialize;
use shared::{Classification, ImageId, PruneConfig, RejectionCategories, RejectionCategory};
use skeet_store::{AppraisalsSource, Band, Images, StoreArgs};
use tracing::info;

const BATCH_SIZE: usize = 10;

#[derive(Parser)]
#[command(about = "Evaluate classifier precision/recall against manually appraised images")]
struct Args {
    #[command(flatten)]
    store: StoreArgs,

    /// Path to prune.toml config file
    #[arg(long)]
    config_path: PathBuf,

    /// Write precision/recall summary as CSV to this file
    #[arg(long)]
    output_csv: Option<PathBuf>,

    /// Rejection categories to enable (comma-separated, defaults to RejectionCategories::default())
    #[arg(long, value_delimiter = ',')]
    categories: Option<Vec<RejectionCategory>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");
    info!(git_hash = env!("BUILD_GIT_HASH"), "eval starting");

    let args = Args::parse();

    let store = args.store.open_store("eval").await?;
    let categories = args.categories.map(RejectionCategories::from);
    let prune_config = PruneConfig::from_file(&args.config_path, categories)?;
    let detector = FaceDetector::from_bundled_weights();
    let text_detector = if prune_config.is_category_enabled(RejectionCategory::Text) {
        info!("text detection enabled, loading models");
        Some(text_detection::TextDetector::from_bundled_models()?)
    } else {
        None
    };

    let appraisals = store.image_appraisals().list_all().await?;
    info!(count = appraisals.len(), "loaded image appraisals");

    let ground_truth: HashMap<ImageId, bool> = appraisals
        .into_iter()
        .map(|(id, appraisal)| (id, appraisal.band == Band::Low))
        .collect();

    let ids: Vec<ImageId> = ground_truth.keys().cloned().collect();
    let mut matrix = ConfusionMatrix::default();
    let mut not_found = 0usize;

    for chunk in ids.chunks(BATCH_SIZE) {
        let images = store.get_by_ids(chunk).await?;
        not_found += chunk.len() - images.len();

        for stored in images.values() {
            let Some(&should_be_pruned) = ground_truth.get(&stored.summary.image_id) else {
                continue;
            };
            let faces = detector.detect(&stored.image);
            let skin_mask = skin_detection::detect_skin(&stored.image);
            let text_area_pct = text_detector.as_ref().and_then(|td| {
                let r = td.detect(&stored.image);
                shared::Percentage::new(
                    r.text_area_pct(stored.image.width(), stored.image.height()),
                )
                .ok()
            });
            let classification = skeet_prune::classify(
                &faces,
                &stored.image,
                &skin_mask,
                text_area_pct,
                &prune_config,
            );
            let was_pruned = matches!(classification, Classification::Rejected(_));
            matrix.record(should_be_pruned, was_pruned);
        }
    }

    if not_found > 0 {
        eprintln!("Warning: {not_found} appraised image(s) not found in store (skipped)");
    }

    print_table(&matrix);

    if let Some(csv_path) = &args.output_csv {
        write_csv(csv_path, &matrix)?;
        eprintln!("CSV written to {}", csv_path.display());
    }

    Ok(())
}

fn print_table(m: &ConfusionMatrix) {
    println!("Evaluation against {} manually appraised images", m.total());
    println!();
    println!("Confusion matrix (positive = should be pruned):");
    println!(
        "  {:<20}  {:>16}  {:>16}",
        "", "Predicted: keep", "Predicted: prune"
    );
    println!(
        "  {:<20}  {:>16}  {:>16}",
        "Actual: keep", m.true_neg, m.false_pos
    );
    println!(
        "  {:<20}  {:>16}  {:>16}",
        "Actual: prune", m.false_neg, m.true_pos
    );
    println!();
    println!(
        "  TP={tp}  FP={fp}  TN={tn}  FN={fn_}",
        tp = m.true_pos,
        fp = m.false_pos,
        tn = m.true_neg,
        fn_ = m.false_neg,
    );
    println!(
        "  Precision={p}  Recall={r}  F1={f1}",
        p = fmt_opt(m.precision()),
        r = fmt_opt(m.recall()),
        f1 = fmt_opt(m.f1()),
    );
}

fn fmt_opt(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "n/a".into(), |v| v.to_string())
}

#[derive(Serialize)]
struct EvalRow {
    tp: u64,
    fp: u64,
    tn: u64,
    #[serde(rename = "fn")]
    fn_: u64,
    precision: Option<Precision>,
    recall: Option<Recall>,
    f1: Option<F1>,
}

fn write_csv(
    path: &std::path::Path,
    m: &ConfusionMatrix,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.serialize(EvalRow {
        tp: m.true_pos,
        fp: m.false_pos,
        tn: m.true_neg,
        fn_: m.false_neg,
        precision: m.precision(),
        recall: m.recall(),
        f1: m.f1(),
    })?;
    wtr.flush()?;
    Ok(())
}
