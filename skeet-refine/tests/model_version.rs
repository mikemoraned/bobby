#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use skeet_refine::model::load_model;

#[test]
fn model_version() {
    let model_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/refine.toml");
    let model = load_model(&model_path)
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", model_path.display()));

    let actual = model.version();
    // Placeholder — will be updated after first run
    let expected = "ea219ee0";
    assert_eq!(
        actual.as_str(),
        expected,
        "model version mismatch: got \"{actual}\". \
         If refine.toml changed intentionally, update the expected value in this test"
    );
}
