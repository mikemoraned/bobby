#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use shared::{Label, RefineModels};

#[test]
fn load_refine_models_and_resolve_production() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/refine.toml");
    let models = RefineModels::load(&path)
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()));

    let production = models
        .by_label(&Label::production())
        .expect("production label must resolve to a model");

    assert_eq!(
        production.version().as_str(),
        "34d8bec0",
        "production model version mismatch: got \"{}\". \
         If refine.toml changed intentionally, update both the production label and this test",
        production.version()
    );
}
