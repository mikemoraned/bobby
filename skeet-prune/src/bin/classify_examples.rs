#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use face_detection::FaceDetector;
use shared::{Classification, PruneConfig};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let examples_dir = root.join("examples");

    let config = PruneConfig::from_file(&root.join("config/prune.toml"))
        .expect("load prune.toml");

    let detector = FaceDetector::from_bundled_weights();

    let mut entries: Vec<_> = std::fs::read_dir(&examples_dir)
        .expect("read examples dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "png" || ext == "jpg" || ext == "jpeg")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let path = entry.path();
        let filename = path.file_name().expect("filename").to_string_lossy();

        let img = image::open(&path).unwrap_or_else(|e| panic!("failed to load {filename}: {e}"));

        println!("{filename}");
        println!("  image: {}x{}", img.width(), img.height());

        let faces = detector.detect(&img);

        if faces.is_empty() {
            println!("  no faces detected");
            println!();
            continue;
        }

        let skin_mask = skin_detection::detect_skin(&img);

        for (i, face) in faces.iter().enumerate() {
            let pct = face.area_pct(img.width(), img.height());
            let frontal = face.is_frontal();

            let face_skin = skin_detection::skin_pct_in_rect(
                &skin_mask,
                face.x as u32,
                face.y as u32,
                face.width as u32,
                face.height as u32,
            );
            let outside_skin = skin_detection::skin_pct_outside_rect(
                &skin_mask,
                face.x as u32,
                face.y as u32,
                face.width as u32,
                face.height as u32,
            );

            println!(
                "  face {i}: score={:.3}, frontal={frontal}, area={pct}, bbox=({:.0}, {:.0}, {:.0}x{:.0}), face_skin={face_skin:.1}%, outside_skin={outside_skin:.1}%",
                face.score, face.x, face.y, face.width, face.height
            );
        }

        let classification = skeet_prune::classify(&faces, &img, &skin_mask, &config);
        match &classification {
            Classification::Accepted(zone) => println!("  classification: Accepted({zone})"),
            Classification::Rejected(reasons) if reasons.is_empty() => {
                println!("  classification: Rejected (no frontal face)");
            }
            Classification::Rejected(reasons) => {
                let reasons_str: Vec<_> = reasons.iter().map(ToString::to_string).collect();
                println!("  classification: Rejected({})", reasons_str.join(", "));
            }
        }
        println!();
    }
}
