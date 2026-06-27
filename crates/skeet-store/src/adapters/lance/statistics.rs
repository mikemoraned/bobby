use std::sync::Arc;

use arrow_array::{RecordBatch, TimestampMicrosecondArray, UInt64Array};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lancedb::query::QueryBase;
use tracing::instrument;

use super::arrow::{micros_to_datetime, typed_column};
use super::decode::decode_rows;
use super::query::{col_in_micros_range, execute_query};
use super::schema::{TableName, prune_stats_v1_schema};
use crate::{PruneStats, SkeetStore, Statistics, StoreError};

/// The `PruneStats` ⟷ Arrow mapping for the `prune_stats` table, both
/// directions kept together: [`to_batch`](Self::to_batch) encodes one record for
/// writing; [`extract`](Self::extract) + [`row`](Self::row) decode rows back.
struct PruneStatsColumns<'a> {
    interval_starts: &'a TimestampMicrosecondArray,
    interval_ends: &'a TimestampMicrosecondArray,
    skeets_seen: &'a UInt64Array,
    images_examined: &'a UInt64Array,
    images_saved: &'a UInt64Array,
}

impl<'a> PruneStatsColumns<'a> {
    fn to_batch(stats: &PruneStats) -> Result<RecordBatch, StoreError> {
        let utc_micros = |dt: DateTime<Utc>| {
            Arc::new(TimestampMicrosecondArray::from(vec![dt.timestamp_micros()]).with_timezone("UTC"))
        };
        Ok(RecordBatch::try_new(
            prune_stats_v1_schema(),
            vec![
                utc_micros(stats.interval_start),
                utc_micros(stats.interval_end),
                Arc::new(UInt64Array::from(vec![stats.skeets_seen])),
                Arc::new(UInt64Array::from(vec![stats.images_examined])),
                Arc::new(UInt64Array::from(vec![stats.images_saved])),
            ],
        )?)
    }

    fn extract(batch: &'a RecordBatch) -> Result<Self, StoreError> {
        Ok(Self {
            interval_starts: typed_column(batch, "interval_start")?,
            interval_ends: typed_column(batch, "interval_end")?,
            skeets_seen: typed_column(batch, "skeets_seen")?,
            images_examined: typed_column(batch, "images_examined")?,
            images_saved: typed_column(batch, "images_saved")?,
        })
    }

    fn row(&self, i: usize) -> PruneStats {
        PruneStats {
            interval_start: micros_to_datetime(self.interval_starts.value(i)),
            interval_end: micros_to_datetime(self.interval_ends.value(i)),
            skeets_seen: self.skeets_seen.value(i),
            images_examined: self.images_examined.value(i),
            images_saved: self.images_saved.value(i),
        }
    }
}

#[async_trait]
impl Statistics for SkeetStore {
    #[instrument(skip(self, stats))]
    async fn record(&self, stats: &PruneStats) -> Result<(), StoreError> {
        self.table(TableName::PruneStats)
            .add(vec![PruneStatsColumns::to_batch(stats)?])
            .write_options(self.write_options())
            .execute()
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    async fn latest_interval_end(&self) -> Result<Option<DateTime<Utc>>, StoreError> {
        let query = self
            .table(TableName::PruneStats)
            .query()
            .select(lancedb::query::Select::columns(&["interval_end"]));
        let batches = execute_query(&query, "latest_interval_end").await?;
        let ends = decode_rows(
            &batches,
            |batch| typed_column::<TimestampMicrosecondArray>(batch, "interval_end"),
            |col, i| Ok(col.value(i)),
        )?;
        Ok(ends.into_iter().max().map(micros_to_datetime))
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
            .only_if_expr(col_in_micros_range(
                "interval_start",
                start.timestamp_micros(),
                end.timestamp_micros(),
            ));
        let batches = execute_query(&query, "interval_counts").await?;

        let rows = decode_rows(&batches, PruneStatsColumns::extract, |cols, i| {
            Ok(cols.row(i))
        })?;

        // The counts sum and the bounds widen to the covered span. There's no
        // identity for the bounds' min/max, so reduce from a real row; an empty
        // window has no covered span, so fall back to the queried bounds.
        Ok(rows
            .into_iter()
            .reduce(|mut acc, row| {
                acc += &row;
                acc
            })
            .unwrap_or_else(|| PruneStats {
                interval_start: start,
                interval_end: end,
                ..PruneStats::default()
            }))
    }
}
