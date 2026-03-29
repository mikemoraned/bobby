//! Loads every blocklist entry's `getPostThread` JSON and runs it through
//! `check_metadata`, asserting that the metadata labels alone are sufficient
//! to block it. This ensures the production code path (which fetches
//! `getPostThread` after image classification) will catch these cases without
//! needing an explicit blocklist lookup.

#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use libtest_mimic::{Arguments, Trial};
use shared::BlocklistConfig;
use skeet_prune::prune_meta_stage::{MetaFilterOutcome, check_metadata};

fn main() {
    let args = Arguments::from_args();

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let blocklist_dir = root.join("blocklist");

    let config = BlocklistConfig::from_file(&blocklist_dir.join("blocklist.toml"))
        .unwrap_or_else(|e| panic!("failed to load blocklist.toml: {e}"));

    let mut trials = Vec::new();

    for entry in &config.blocked {
        let rkey = entry.skeet_id.rkey().to_string();
        let json_path = blocklist_dir.join(format!("{rkey}.json"));
        let at_uri = entry.skeet_id.to_string();

        trials.push(Trial::test(
            format!("{rkey}::blocked_by_meta_filter"),
            move || {
                let json_text = std::fs::read_to_string(&json_path).map_err(|e| {
                    format!(
                        "missing JSON for {at_uri} at {}: {e}",
                        json_path.display()
                    )
                })?;

                let json: serde_json::Value =
                    serde_json::from_str(&json_text).map_err(|e| format!("invalid JSON: {e}"))?;

                let outcome = check_metadata(&json);

                match outcome {
                    MetaFilterOutcome::Blocked(_) => Ok(()),
                    MetaFilterOutcome::Pass => Err(format!(
                        "{at_uri} should be blocked by meta filter but was not"
                    )
                    .into()),
                }
            },
        ));
    }

    libtest_mimic::run(&args, trials).exit();
}
