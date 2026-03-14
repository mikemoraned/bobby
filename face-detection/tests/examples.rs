#![warn(clippy::all, clippy::nursery)]

use std::cell::RefCell;
use std::path::Path;

use face_detection::{FaceDetector, Quadrant, face_quadrant};
use libtest_mimic::{Arguments, Trial};
use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    example: Vec<Example>,
}

#[derive(Deserialize)]
struct Example {
    path: String,
    archetype: Option<String>,
}

thread_local! {
    static DETECTOR: RefCell<Option<FaceDetector>> = const { RefCell::new(None) };
}

fn with_detector<R>(f: impl FnOnce(&FaceDetector) -> R) -> R {
    DETECTOR.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            *opt = Some(FaceDetector::from_bundled_weights());
        }
        f(opt.as_ref().expect("detector initialized above"))
    })
}

fn parse_archetype(s: &str) -> Quadrant {
    match s {
        "TOP_LEFT" => Quadrant::TopLeft,
        "TOP_RIGHT" => Quadrant::TopRight,
        "BOTTOM_LEFT" => Quadrant::BottomLeft,
        "BOTTOM_RIGHT" => Quadrant::BottomRight,
        other => panic!("unknown archetype in config: {other}"),
    }
}

/// Classify an image: detect faces, pick the best frontal face, return its quadrant.
/// Returns None if no frontal face is found.
fn classify(detector: &FaceDetector, img: &image::DynamicImage) -> Option<Quadrant> {
    let faces = detector.detect(img);
    let face = faces.iter().find(|f| f.is_frontal())?;
    Some(face_quadrant(face, img.width(), img.height()))
}

fn main() {
    let args = Arguments::from_args();

    let config_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/expected.toml");
    let config_text = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", config_path.display()));
    let config: Config =
        toml::from_str(&config_text).unwrap_or_else(|e| panic!("failed to parse config: {e}"));

    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");

    let mut trials = Vec::new();

    for example in &config.example {
        let stem = Path::new(&example.path)
            .file_stem()
            .expect("example path should have a file stem")
            .to_string_lossy()
            .to_string();
        let img_path = examples_dir.join(&example.path);
        let expected = example.archetype.as_deref().map(parse_archetype);

        trials.push(Trial::test(format!("{stem}::archetype"), move || {
            let img = image::open(&img_path)
                .map_err(|e| format!("failed to load {}: {e}", img_path.display()))?;
            let actual = with_detector(|d| classify(d, &img));
            if actual != expected {
                let fmt = |q: Option<Quadrant>| match q {
                    Some(q) => format!("Some({q})"),
                    None => "None".to_string(),
                };
                return Err(format!("expected {}, got {}", fmt(expected), fmt(actual)).into());
            }
            Ok(())
        }));
    }

    libtest_mimic::run(&args, trials).exit();
}
