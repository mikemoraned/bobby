use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use shared::Rejection;

pub fn create_status() -> ProgressBar {
    let status = ProgressBar::new_spinner();
    #[allow(clippy::literal_string_with_formatting_args)]
    let style = ProgressStyle::with_template("{elapsed_precise} {spinner} {msg}")
        .expect("valid template")
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏-");
    status.set_style(style);
    status.enable_steady_tick(Duration::from_millis(100));
    status.set_message("connected, listening for posts...");
    status
}

pub fn update_status(
    status: &ProgressBar,
    posts: u64,
    images: u64,
    saved: u64,
    rejected: u64,
    rejection_reasons: &HashMap<Rejection, u64>,
) {
    let hit_rate = if images > 0 {
        (saved as f64 / images as f64) * 100.0
    } else {
        0.0
    };

    let mut msg = format!(
        "skeets: {posts} | images: {images} | saved: {saved} ({hit_rate:.1}%) | rejected: {rejected}"
    );

    if !rejection_reasons.is_empty() {
        let total_reasons: u64 = rejection_reasons.values().sum();
        let mut sorted: Vec<_> = rejection_reasons.iter().collect();
        sorted.sort_by_key(|(r, _)| r.to_string());

        write!(msg, " (").expect("write to String");
        for (i, (reason, count)) in sorted.iter().enumerate() {
            let pct = (**count as f64 / total_reasons as f64) * 100.0;
            if i > 0 {
                write!(msg, ", ").expect("write to String");
            }
            write!(msg, "{reason}: {count} [{pct:.0}%]").expect("write to String");
        }
        write!(msg, ")").expect("write to String");
    }

    status.set_message(msg);
}
