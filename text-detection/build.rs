// Build script: panicking with a descriptive message is the idiomatic failure mode, and
// `expect` keeps the precise reason (which env var / which directory) that a bare `?` on
// `VarError`/`io::Error` would drop.
#[allow(clippy::expect_used)]
fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let models_dir = std::path::Path::new(&manifest_dir)
        .join("../models")
        .canonicalize()
        .expect("models directory not found");
    let detection = models_dir.join("text-detection.rten");
    let recognition = models_dir.join("text-recognition.rten");
    println!(
        "cargo:rustc-env=TEXT_DETECTION_MODEL_PATH={}",
        detection.display()
    );
    println!(
        "cargo:rustc-env=TEXT_RECOGNITION_MODEL_PATH={}",
        recognition.display()
    );
}
