#![warn(clippy::all, clippy::nursery)]

use std::cell::RefCell;
use std::path::Path;

use face_detection::{FaceDetector, classify};
use libtest_mimic::{Arguments, Trial};
use serde::Deserialize;
use shared::{ArchetypeConfig, Classification, Quadrant, Rejection};

#[derive(Deserialize)]
struct ExpectedConfig {
    example: Vec<Example>,
}

#[derive(Deserialize)]
struct Example {
    path: String,
    archetype: Option<String>,
    #[serde(default)]
    rejected: Vec<String>,
}

thread_local! {
    static DETECTOR: RefCell<Option<FaceDetector>> = const { RefCell::new(None) };
    static TEXT_DETECTOR: RefCell<Option<text_detection::TextDetector>> = const { RefCell::new(None) };
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

fn with_text_detector<R>(f: impl FnOnce(&text_detection::TextDetector) -> R) -> R {
    TEXT_DETECTOR.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            *opt = Some(text_detection::TextDetector::from_bundled_models());
        }
        f(opt.as_ref().expect("text detector initialized above"))
    })
}

fn expected_classification(example: &Example) -> Classification {
    if let Some(archetype) = &example.archetype {
        let quadrant: Quadrant = archetype.parse().unwrap_or_else(|e| panic!("{e}"));
        Classification::Accepted(quadrant)
    } else {
        let reasons = example
            .rejected
            .iter()
            .map(|s| s.parse::<Rejection>().unwrap_or_else(|e| panic!("{e}")))
            .collect();
        Classification::Rejected(reasons)
    }
}

fn main() {
    let args = Arguments::from_args();

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");

    let config = ArchetypeConfig::from_file(&root.join("shared/archetype.toml"))
        .unwrap_or_else(|e| panic!("failed to load archetype.toml: {e}"));

    let expected_text = std::fs::read_to_string(root.join("examples/expected.toml"))
        .unwrap_or_else(|e| panic!("failed to read expected.toml: {e}"));
    let expected: ExpectedConfig = toml::from_str(&expected_text)
        .unwrap_or_else(|e| panic!("failed to parse expected.toml: {e}"));

    let examples_dir = root.join("examples");

    let mut trials = Vec::new();

    for example in &expected.example {
        let stem = Path::new(&example.path)
            .file_stem()
            .expect("example path should have a file stem")
            .to_string_lossy()
            .to_string();
        let img_path = examples_dir.join(&example.path);
        let expected = expected_classification(example);

        trials.push(Trial::test(format!("{stem}::classification"), move || {
            let img = image::open(&img_path)
                .map_err(|e| format!("failed to load {}: {e}", img_path.display()))?;
            let skin_mask = skin_detection::detect_skin(&img);
            let word_count = with_text_detector(|td| td.count_characters(&img));
            let actual = with_detector(|d| classify(d, &img, &skin_mask, word_count, &config));
            if actual != expected {
                return Err(format!("expected {expected:?}, got {actual:?}").into());
            }
            Ok(())
        }));
    }

    libtest_mimic::run(&args, trials).exit();
}
