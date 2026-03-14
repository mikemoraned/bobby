#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use face_detection::FaceDetector;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let examples_dir = root.join("examples");

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

        for (i, face) in faces.iter().enumerate() {
            let pct = face.area_pct(img.width(), img.height());
            let frontal = face.is_frontal();
            println!(
                "  face {i}: score={:.3}, frontal={frontal}, area={pct}, bbox=({:.0}, {:.0}, {:.0}x{:.0})",
                face.score, face.x, face.y, face.width, face.height
            );
        }
        println!();
    }
}
