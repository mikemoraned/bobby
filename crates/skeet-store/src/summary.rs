use std::fmt;

use crate::types::{DiscoveredAt, OriginalAt};

pub struct SkeetStoreSummary {
    pub image_count: usize,
    pub score_count: usize,
    pub scored_image_count: usize,
    pub discovered_at_range: Option<(DiscoveredAt, DiscoveredAt)>,
    pub original_at_range: Option<(OriginalAt, OriginalAt)>,
}

impl fmt::Display for SkeetStoreSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Images:        {}", self.image_count)?;
        writeln!(f, "Scores:        {}", self.score_count)?;
        writeln!(f, "Scored images: {}", self.scored_image_count)?;
        if let Some((min, max)) = &self.discovered_at_range {
            writeln!(f, "Discovered at: {min} .. {max}")?;
        } else {
            writeln!(f, "Discovered at: (none)")?;
        }
        if let Some((min, max)) = &self.original_at_range {
            write!(f, "Original at:   {min} .. {max}")?;
        } else {
            write!(f, "Original at:   (none)")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_shows_counts() {
        let summary = SkeetStoreSummary {
            image_count: 42,
            score_count: 7,
            scored_image_count: 3,
            discovered_at_range: None,
            original_at_range: None,
        };
        let s = summary.to_string();
        assert!(s.contains("42"));
        assert!(s.contains("7"));
        assert!(s.contains("3"));
    }
}
