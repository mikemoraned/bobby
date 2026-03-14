#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use shared::ArchetypeConfig;

#[test]
fn config_version() {
    let config_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("archetype.toml");
    let config = ArchetypeConfig::from_file(&config_path)
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", config_path.display()));

    let actual = config.version();
    let expected = "ae4f68bf";
    assert_eq!(
        actual.as_str(),
        expected,
        "config version mismatch: got \"{actual}\". \
         If the config changed intentionally, update the expected value in this test"
    );
}
