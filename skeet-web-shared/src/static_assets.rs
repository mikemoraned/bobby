//! Vendored web static assets shared across web-facing crates.
//!
//! Currently this is just `htmx.min.js`. Consumer crates should call
//! [`web_static_files`] from their [`cot::App::static_files`] implementation
//! to register these files with the project's static files middleware.

use cot::static_files::StaticFile;

/// Returns the list of static files shared across web crates.
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
