use std::fmt;

use arrow_array::StringArray;
use lancedb::query::QueryBase;
use tracing::instrument;

use crate::arrow_utils::{min_max_timestamp, typed_column};
use crate::error::StoreError;
use crate::lancedb_utils::execute_query;
use crate::types::{DiscoveredAt, OriginalAt};
use crate::SkeetStore;

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

impl SkeetStore {
    #[instrument(skip(self))]
    pub async fn summarise(&self) -> Result<SkeetStoreSummary, StoreError> {
        let image_count = self.images_table.count_rows(None).await?;
        let score_count = self.scores_table.count_rows(None).await?;

        let timestamps_query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "discovered_at",
                "original_at",
            ]));
        let batches = execute_query(&timestamps_query, "summarise:timestamps").await?;

        let discovered_at_range = min_max_timestamp(&batches, "discovered_at")?
            .map(|(min, max)| (DiscoveredAt::new(min), DiscoveredAt::new(max)));
        let original_at_range = min_max_timestamp(&batches, "original_at")?
            .map(|(min, max)| (OriginalAt::new(min), OriginalAt::new(max)));

        let scored_query = self
            .scores_table
            .query()
            .select(lancedb::query::Select::columns(&["image_id"]));
        let scored_batches = execute_query(&scored_query, "summarise:scored_ids").await?;

        let mut scored_ids = std::collections::HashSet::new();
        for batch in &scored_batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            for i in 0..batch.num_rows() {
                scored_ids.insert(image_ids.value(i).to_string());
            }
        }

        Ok(SkeetStoreSummary {
            image_count,
            score_count,
            scored_image_count: scored_ids.len(),
            discovered_at_range,
            original_at_range,
        })
    }
}
