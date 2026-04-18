#![warn(clippy::all, clippy::nursery)]

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use clap::Parser;
use face_detection::FaceDetector;
use shared::{Classification, PruneConfig};
use skeet_store::{Band, ImageId, StoreArgs};
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
}

#[derive(Default)]
struct ConfusionMatrix {
    true_pos: u64,
    false_pos: u64,
    true_neg: u64,
    false_neg: u64,
}

impl ConfusionMatrix {
    const fn record(&mut self, should_be_pruned: bool, was_pruned: bool) {
        match (should_be_pruned, was_pruned) {
            (true, true) => self.true_pos += 1,
            (false, true) => self.false_pos += 1,
            (false, false) => self.true_neg += 1,
            (true, false) => self.false_neg += 1,
        }
    }

    fn precision(&self) -> f64 {
        let denom = self.true_pos + self.false_pos;
        if denom == 0 {
            0.0
        } else {
            self.true_pos as f64 / denom as f64
        }
    }

    fn recall(&self) -> f64 {
        let denom = self.true_pos + self.false_neg;
        if denom == 0 {
            0.0
        } else {
            self.true_pos as f64 / denom as f64
        }
    }

    fn f1(&self) -> f64 {
        let p = self.precision();
        let r = self.recall();
        let denom = p + r;
        if denom == 0.0 {
            0.0
        } else {
            2.0 * p * r / denom
        }
    }

    const fn total(&self) -> u64 {
        self.true_pos + self.false_pos + self.true_neg + self.false_neg
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    shared::tracing::init("info");

    let args = Args::parse();

    let store = args.store.open_store().await?;
    let prune_config = PruneConfig::from_file(&args.config_path)?;
    let detector = FaceDetector::from_bundled_weights();

    let appraisals = store.list_all_image_appraisals().await?;
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

        for stored in &images {
            let Some(&should_be_pruned) = ground_truth.get(&stored.summary.image_id) else {
                continue;
            };
            let faces = detector.detect(&stored.image);
            let skin_mask = skin_detection::detect_skin(&stored.image);
            let classification =
                skeet_prune::classify(&faces, &stored.image, &skin_mask, &prune_config);
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
        "  Precision={p:.3}  Recall={r:.3}  F1={f1:.3}",
        p = m.precision(),
        r = m.recall(),
        f1 = m.f1(),
    );
}

fn write_csv(
    path: &std::path::Path,
    m: &ConfusionMatrix,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    writeln!(w, "tp,fp,tn,fn,precision,recall,f1")?;
    writeln!(
        w,
        "{tp},{fp},{tn},{fn_},{p:.3},{r:.3},{f1:.3}",
        tp = m.true_pos,
        fp = m.false_pos,
        tn = m.true_neg,
        fn_ = m.false_neg,
        p = m.precision(),
        r = m.recall(),
        f1 = m.f1(),
    )?;
    Ok(())
}
