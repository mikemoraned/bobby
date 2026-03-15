#![warn(clippy::all, clippy::nursery)]

use std::path::Path;

use libtest_mimic::{Arguments, Trial};
use shared::BlocklistConfig;

fn main() {
    let args = Arguments::from_args();

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let blocklist_dir = root.join("blocklist");

    let config = BlocklistConfig::from_file(&blocklist_dir.join("blocklist.toml"))
        .unwrap_or_else(|e| panic!("failed to load blocklist.toml: {e}"));

    let mut trials = Vec::new();

    for entry in &config.blocked {
        let rkey = entry
            .at_uri
            .rsplit('/')
            .next()
            .expect("at:// URI should have an rkey")
            .to_string();
        let json_path = blocklist_dir.join(format!("{rkey}.json"));
        let at_uri = entry.at_uri.clone();

        trials.push(Trial::test(
            format!("{rkey}::should_be_blocked"),
            move || {
                let json_text = std::fs::read_to_string(&json_path).map_err(|e| {
                    format!(
                        "missing JSON for {at_uri} at {}: {e}. Run: cargo run -p skeet-finder --bin add-to-blocklist -- \"{at_uri}\"",
                        json_path.display()
                    )
                })?;

                let json: serde_json::Value =
                    serde_json::from_str(&json_text).map_err(|e| format!("invalid JSON: {e}"))?;

                let blocked = skeet_finder::content_filter::blocked_labels(&json);

                if blocked.is_empty() {
                    return Err(format!(
                        "{at_uri} should be blocked but no blocked labels found"
                    )
                    .into());
                }

                Ok(())
            },
        ));
    }

    libtest_mimic::run(&args, trials).exit();
}
