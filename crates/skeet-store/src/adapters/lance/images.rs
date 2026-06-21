use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{LargeBinaryArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use async_trait::async_trait;
use lancedb::query::QueryBase;
use shared::{DiscoveredAt, ImageId, SkeetId};
use tracing::instrument;

use super::arrow::{encode_image_as_png, typed_column};
use super::decode::{batches_to_original_images, batches_to_stored_images, batches_to_summaries};
use super::query::execute_query;
use super::schema::images_v6_schema;
use crate::{
    ImageRecord, Images, SkeetStore, StoreError, StoredImage, StoredImageSummary, StoredOriginal,
};

const SUMMARY_COLUMNS: &[&str] = &[
    "image_id",
    "skeet_id",
    "discovered_at",
    "original_at",
    "archetype",
    "config_version",
    "detected_text",
];

#[async_trait]
impl Images for SkeetStore {
    #[instrument(skip(self, record), fields(image_id = %record.image_id, skeet_id = %record.skeet_id))]
    async fn add(&self, record: &ImageRecord) -> Result<(), StoreError> {
        let schema = images_v6_schema();

        let image_bytes = encode_image_as_png(&record.image)?;
        let annotated_bytes = encode_image_as_png(&record.annotated_image)?;
        let image_id_str = record.image_id.to_string();
        let skeet_id_str = record.skeet_id.to_string();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![image_id_str.as_str()])),
                Arc::new(StringArray::from(vec![skeet_id_str.as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&image_bytes])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.discovered_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![record.original_at.timestamp_micros()])
                        .with_timezone("UTC"),
                ),
                Arc::new(StringArray::from(vec![record.zone.to_string().as_str()])),
                Arc::new(LargeBinaryArray::from_vec(vec![&annotated_bytes])),
                Arc::new(StringArray::from(vec![
                    record.config_version.to_string().as_str(),
                ])),
                Arc::new(StringArray::from(vec![record.detected_text.as_str()])),
            ],
        )?;

        self.images_table
            .add(vec![batch])
            .write_options(self.write_options())
            .execute()
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_by_id(&self, image_id: &ImageId) -> Result<Option<StoredImage>, StoreError> {
        let query = self
            .images_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1);
        let batches = execute_query(&query, "get_by_id").await?;
        Ok(batches_to_stored_images(&batches)?.into_iter().next())
    }

    #[instrument(skip(self, image_ids), fields(count = image_ids.len()))]
    async fn get_by_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, StoredImage>, StoreError> {
        if image_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let query = self
            .images_table
            .query()
            .only_if(id_in_list_filter(image_ids));
        let batches = execute_query(&query, "get_by_ids").await?;
        Ok(batches_to_stored_images(&batches)?
            .into_iter()
            .map(|s| (s.summary.image_id.clone(), s))
            .collect())
    }

    #[instrument(skip(self, image_ids), fields(count = image_ids.len()))]
    async fn get_originals_by_ids(
        &self,
        image_ids: &[ImageId],
    ) -> Result<HashMap<ImageId, StoredOriginal>, StoreError> {
        if image_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let query = self
            .images_table
            .query()
            .only_if(id_in_list_filter(image_ids))
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "skeet_id",
                "discovered_at",
                "original_at",
                "archetype",
                "config_version",
                "detected_text",
                "image",
            ]));
        let batches = execute_query(&query, "get_originals_by_ids").await?;
        Ok(batches_to_original_images(&batches)?
            .into_iter()
            .map(|o| (o.summary.image_id.clone(), o))
            .collect())
    }

    #[instrument(skip(self))]
    async fn exists(&self, image_id: &ImageId) -> Result<bool, StoreError> {
        let query = self
            .images_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .select(lancedb::query::Select::columns(&["image_id"]))
            .limit(1);
        let batches = execute_query(&query, "exists").await?;
        Ok(batches.iter().any(|b| b.num_rows() > 0))
    }

    #[instrument(skip(self))]
    async fn delete_by_id(&self, image_id: &ImageId) -> Result<(), StoreError> {
        self.images_table
            .delete(&format!("image_id = '{image_id}'"))
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn count(&self) -> Result<usize, StoreError> {
        Ok(self.images_table.count_rows(None).await?)
    }

    #[instrument(skip(self))]
    async fn list_all_summaries(&self) -> Result<Vec<StoredImageSummary>, StoreError> {
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(SUMMARY_COLUMNS));
        let batches = execute_query(&query, "list_all_summaries").await?;
        batches_to_summaries(&batches)
    }

    #[instrument(skip(self))]
    async fn list_summaries_page(
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

    #[instrument(skip(self))]
    async fn unique_skeet_ids(&self) -> Result<Vec<SkeetId>, StoreError> {
        let query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&["skeet_id"]));
        let batches = execute_query(&query, "unique_skeet_ids").await?;

        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for batch in &batches {
            let skeet_ids = typed_column::<StringArray>(batch, "skeet_id")?;
            for i in 0..batch.num_rows() {
                let id = skeet_ids.value(i).to_string();
                if seen.insert(id.clone()) {
                    ids.push(SkeetId::new(id)?);
                }
            }
        }

        Ok(ids)
    }

    #[instrument(skip(self))]
    async fn list_all_image_ids_by_most_recent(
        &self,
        since: Option<&DiscoveredAt>,
    ) -> Result<Vec<ImageId>, StoreError> {
        let mut query = self
            .images_table
            .query()
            .select(lancedb::query::Select::columns(&[
                "image_id",
                "discovered_at",
            ]));
        if let Some(ts) = since {
            let cutoff_us = ts.timestamp_micros();
            query = query.only_if(format!(
                "discovered_at >= arrow_cast({cutoff_us}, 'Timestamp(Microsecond, Some(\"UTC\"))')"
            ));
        }
        let batches = execute_query(&query, "list_all_image_ids_by_most_recent").await?;

        let mut id_times = Vec::new();
        for batch in &batches {
            let image_ids = typed_column::<StringArray>(batch, "image_id")?;
            let discovered_ats = typed_column::<TimestampMicrosecondArray>(batch, "discovered_at")?;
            for i in 0..batch.num_rows() {
                id_times.push((
                    image_ids.value(i).parse::<ImageId>()?,
                    discovered_ats.value(i),
                ));
            }
        }
        id_times.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(id_times.into_iter().map(|(id, _)| id).collect())
    }
}

fn id_in_list_filter(image_ids: &[ImageId]) -> String {
    let id_list = image_ids
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("image_id IN ({id_list})")
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use shared::DiscoveredAt;

    use crate::Images;
    use crate::test_utils::{make_record_at, open_temp_store};

    fn at_minutes_ago(minutes: i64) -> DiscoveredAt {
        DiscoveredAt::new(Utc::now() - Duration::minutes(minutes))
    }

    #[tokio::test]
    async fn first_page_returns_newest_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_temp_store(&dir).await;

        for i in 0..5u8 {
            let minutes_ago = (4 - i) as i64 * 10;
            let record =
                make_record_at(&format!("p{i}"), i * 40, 0, 0, at_minutes_ago(minutes_ago));
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
            let record =
                make_record_at(&format!("e{i}"), i * 80, 0, 0, at_minutes_ago(i as i64 * 5));
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
            let record =
                make_record_at(&format!("c{i}"), i * 60, 0, 0, at_minutes_ago(minutes_ago));
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
