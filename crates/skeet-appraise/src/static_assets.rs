//! Vendored web static assets served by skeet-feed.
//!
//! Currently this is just `htmx.min.js`. Called from
//! [`cot::App::static_files`] to register these files with the project's
//! static files middleware.

use cot::static_files::StaticFile;

/// Returns the list of static files bundled with skeet-feed.
///
/// Each file is included at compile time via [`include_bytes!`], so the
/// returned vector is cheap to build and never fails at runtime.
#[must_use]
pub fn web_static_files() -> Vec<StaticFile> {
    cot::static_files!("htmx.min.js")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_htmx() {
        let files = web_static_files();
        assert_eq!(files.len(), 1);
    }
}
