//! Cursor-based paging over stored image summaries, ordered by
//! `discovered_at` descending.
//!
//! Cursors are [`DiscoveredAt`] values: passing `before = Some(cursor)`
//! returns rows strictly older than `cursor`. Passing `before = None`
//! returns the newest page.

use lancedb::query::QueryBase;
use tracing::instrument;

use crate::lancedb_utils::execute_query;
use crate::stored::batches_to_summaries;
use crate::{DiscoveredAt, SkeetStore, StoreError, StoredImageSummary};

const SUMMARY_COLUMNS: &[&str] = &[
    "image_id",
    "skeet_id",
    "discovered_at",
    "original_at",
    "archetype",
    "config_version",
    "detected_text",
];

impl SkeetStore {
    /// Return up to `limit` summaries ordered by `discovered_at` desc, starting
    /// strictly before the given cursor (or from the newest row if `before`
    /// is `None`). The second element of the tuple is the cursor to pass as
    /// `before` for the next page, or `None` if there are no more rows.
    #[instrument(skip(self))]
    pub async fn list_summaries_page(
        &self,
        before: Option<DiscoveredAt>,
        limit: usize,
    ) -> Result<(Vec<StoredImageSummary>, Option<DiscoveredAt>), StoreError> {
        if limit == 0 {
            return Ok((Vec::new(), None));
        }

        let mut query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(SUMMARY_COLUMNS));
        if let Some(cursor) = before.as_ref() {
            let cursor_us = cursor.timestamp_micros();
            query = query.only_if(format!(
                "discovered_at < arrow_cast({cursor_us}, 'Timestamp(Microsecond, Some(\"UTC\"))')"
            ));
        }

        let batches = execute_query(&query, "list_summaries_page").await?;
        let mut summaries = batches_to_summaries(&batches)?;
        summaries.sort_by(|a, b| b.discovered_at.cmp(&a.discovered_at));

        let next_cursor = if summaries.len() > limit {
            summaries.truncate(limit);
            summaries.last().map(|s| s.discovered_at.clone())
        } else {
            None
        };

        Ok((summaries, next_cursor))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use crate::test_utils::{make_record_at, open_temp_store};
    use crate::DiscoveredAt;

    fn at_minutes_ago(minutes: i64) -> DiscoveredAt {
        DiscoveredAt::new(Utc::now() - Duration::minutes(minutes))
    }

    #[tokio::test]
    async fn first_page_returns_newest_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        for i in 0..5u8 {
            let minutes_ago = (4 - i) as i64 * 10;
            let record = make_record_at(
                &format!("p{i}"),
                i * 40,
                0,
                0,
                at_minutes_ago(minutes_ago),
            );
            store.add(&record).await.unwrap();
        }

        let (page, cursor) = store.list_summaries_page(None, 3).await.unwrap();
        assert_eq!(page.len(), 3);
        // desc order
        assert!(page[0].discovered_at >= page[1].discovered_at);
        assert!(page[1].discovered_at >= page[2].discovered_at);
        // cursor present because more rows remain
        assert!(cursor.is_some());
    }

    #[tokio::test]
    async fn subsequent_pages_advance_and_end_of_data() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        for i in 0..5u8 {
            let minutes_ago = (4 - i) as i64 * 10;
            let record = make_record_at(
                &format!("s{i}"),
                i * 40,
                i * 10,
                0,
                at_minutes_ago(minutes_ago),
            );
            store.add(&record).await.unwrap();
        }

        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let (page, next) = store.list_summaries_page(cursor, 2).await.unwrap();
            assert!(!page.is_empty());
            for summary in &page {
                seen.push(summary.image_id.clone());
            }
            if next.is_none() {
                break;
            }
            cursor = next;
        }

        assert_eq!(seen.len(), 5, "paging should visit every row exactly once");
        let unique: std::collections::HashSet<_> = seen.iter().cloned().collect();
        assert_eq!(unique.len(), 5, "no duplicates across pages");
    }

    #[tokio::test]
    async fn end_of_data_returns_no_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        for i in 0..2u8 {
            let record = make_record_at(
                &format!("e{i}"),
                i * 80,
                0,
                0,
                at_minutes_ago(i as i64 * 5),
            );
            store.add(&record).await.unwrap();
        }

        let (page, cursor) = store.list_summaries_page(None, 10).await.unwrap();
        assert_eq!(page.len(), 2);
        assert!(cursor.is_none(), "cursor must be None at end of data");
    }

    #[tokio::test]
    async fn concurrent_insert_during_paging_does_not_duplicate_or_skip() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        // Insert 4 rows at distinct, strictly-decreasing ages.
        for i in 0..4u8 {
            let minutes_ago = (3 - i) as i64 * 10 + 30; // 60, 50, 40, 30 min ago
            let record = make_record_at(
                &format!("c{i}"),
                i * 60,
                0,
                0,
                at_minutes_ago(minutes_ago),
            );
            store.add(&record).await.unwrap();
        }

        // Read the first page (2 rows).
        let (page1, cursor) = store.list_summaries_page(None, 2).await.unwrap();
        assert_eq!(page1.len(), 2);
        let cursor = cursor.expect("cursor after first page");

        // Insert a newer row — should not appear in the next page because
        // the cursor is strictly older than anything on page 1.
        let newer = make_record_at("c-newer", 200, 200, 200, at_minutes_ago(5));
        store.add(&newer).await.unwrap();

        let (page2, cursor2) = store.list_summaries_page(Some(cursor), 2).await.unwrap();
        assert_eq!(page2.len(), 2, "second page returns the remaining two rows");
        assert!(cursor2.is_none(), "no more rows older than the cursor");

        // The newly inserted row must not be in page2.
        let page2_ids: std::collections::HashSet<_> =
            page2.iter().map(|s| s.image_id.clone()).collect();
        assert!(!page2_ids.contains(&newer.image_id));
    }
}
