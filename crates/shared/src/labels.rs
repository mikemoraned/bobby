/// Label values that indicate content should be excluded/blocked.
pub const EXCLUDED_VALUES: &[&str] = &["porn", "sexual", "nudity", "!no-unauthenticated"];

/// A moderation label value attached to a post, its author, or a quoted record
/// (e.g. `porn`, `!no-unauthenticated`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModerationLabel(String);

impl ModerationLabel {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModerationLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
