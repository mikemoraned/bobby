fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    println!(
        "cargo:rustc-env=TEXT_DETECTION_MODEL_PATH={manifest_dir}/../models/text-detection.rten"
    );
    println!(
        "cargo:rustc-env=TEXT_RECOGNITION_MODEL_PATH={manifest_dir}/../models/text-recognition.rten"
    );
}
