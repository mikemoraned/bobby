use std::sync::Arc;

use arrow_array::{RecordBatch, TimestampMicrosecondArray, UInt64Array};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lancedb::query::{QueryBase, Select};
use tracing::instrument;

use super::arrow::typed_column;
use super::decode::decode_rows;
use super::query::{col_in_micros_range, execute_query};
use super::schema::{TableName, prune_stats_v1_schema};
use crate::{PruneStats, SkeetStore, Statistics, StoreError};

#[async_trait]
impl Statistics for SkeetStore {
    #[instrument(skip(self, stats))]
    async fn record(&self, stats: &PruneStats) -> Result<(), StoreError> {
        let schema = prune_stats_v1_schema();
        let utc_micros = |dt: DateTime<Utc>| {
            Arc::new(TimestampMicrosecondArray::from(vec![dt.timestamp_micros()]).with_timezone("UTC"))
        };
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                utc_micros(stats.interval_start),
                utc_micros(stats.interval_end),
                Arc::new(UInt64Array::from(vec![stats.skeets_seen])),
                Arc::new(UInt64Array::from(vec![stats.images_examined])),
                Arc::new(UInt64Array::from(vec![stats.images_saved])),
            ],
        )?;

        self.table(TableName::PruneStats)
            .add(vec![batch])
            .write_options(self.write_options())
            .execute()
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn interval_counts(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<PruneStats, StoreError> {
        let query = self
            .table(TableName::PruneStats)
            .query()
            .select(Select::columns(&[
                "interval_start",
                "skeets_seen",
                "images_examined",
                "images_saved",
            ]))
            .only_if_expr(col_in_micros_range(
                "interval_start",
                start.timestamp_micros(),
                end.timestamp_micros(),
            ));
        let batches = execute_query(&query, "interval_counts").await?;

        let rows = decode_rows(
            &batches,
            |batch| {
                Ok((
                    typed_column::<UInt64Array>(batch, "skeets_seen")?,
                    typed_column::<UInt64Array>(batch, "images_examined")?,
                    typed_column::<UInt64Array>(batch, "images_saved")?,
                ))
            },
            |(skeets, examined, saved), i| {
                Ok((skeets.value(i), examined.value(i), saved.value(i)))
            },
        )?;

        let mut totals = PruneStats {
            interval_start: start,
            interval_end: end,
            skeets_seen: 0,
            images_examined: 0,
            images_saved: 0,
        };
        for (skeets, examined, saved) in rows {
            totals.skeets_seen = totals.skeets_seen.saturating_add(skeets);
            totals.images_examined = totals.images_examined.saturating_add(examined);
            totals.images_saved = totals.images_saved.saturating_add(saved);
        }
        Ok(totals)
    }
}
