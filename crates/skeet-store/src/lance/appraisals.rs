use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchIterator, StringArray, TimestampMicrosecondArray};
use chrono::Utc;
use lancedb::query::QueryBase;
use shared::{Appraisal, Appraiser, Band, ImageId, SkeetId};
use tracing::instrument;

use super::arrow::typed_column;
use super::query::execute_query;
use super::schema::appraisal_schema;
use crate::{AppraisalSource, SkeetStore, StoreError};

/// A handle to one manual-appraisal table, keyed by `K` (`SkeetId` or `ImageId`).
///
/// The CRUD is written once over both key spaces, which differ only in key type,
/// table, and id-column. Obtain one via [`AppraisalSource`] (e.g.
/// `store.skeet_appraisals()`).
pub struct Appraisals<K> {
    table: lancedb::Table,
    id_column: &'static str,
    _key: PhantomData<K>,
}

impl<K> Appraisals<K> {
    const fn new(table: lancedb::Table, id_column: &'static str) -> Self {
        Self {
            table,
            id_column,
            _key: PhantomData,
        }
    }
}

impl<K: Display + Send + Sync> Appraisals<K> {
    #[instrument(skip_all, fields(table = self.id_column))]
    pub async fn set(&self, id: &K, band: Band, appraiser: &Appraiser) -> Result<(), StoreError> {
        let schema = appraisal_schema(self.id_column);
        let id_str = id.to_string();
        let band_str = band.to_string();
        let appraiser_str = appraiser.to_string();
        let now_us = Utc::now().timestamp_micros();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id_str.as_str()])),
                Arc::new(StringArray::from(vec![band_str.as_str()])),
                Arc::new(StringArray::from(vec![appraiser_str.as_str()])),
                Arc::new(TimestampMicrosecondArray::from(vec![now_us]).with_timezone("UTC")),
            ],
        )?;

        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let mut builder = self.table.merge_insert(&[self.id_column]);
        builder.when_matched_update_all(None);
        builder.when_not_matched_insert_all();
        builder.execute(Box::new(reader)).await?;
        Ok(())
    }

    #[instrument(skip_all, fields(table = self.id_column))]
    pub async fn clear(&self, id: &K) -> Result<(), StoreError> {
        self.table
            .delete(&format!("{} = '{id}'", self.id_column))
            .await?;
        Ok(())
    }

    #[instrument(skip_all, fields(table = self.id_column))]
    pub async fn get(&self, id: &K) -> Result<Option<Appraisal>, StoreError> {
        let query = self
            .table
            .query()
            .only_if(format!("{} = '{id}'", self.id_column))
            .limit(1);
        let batches = execute_query(&query, &format!("appraisal_get:{}", self.id_column)).await?;
        parse_single_appraisal(&batches)
    }
}

impl<K: FromStr + Send + Sync> Appraisals<K>
where
    StoreError: From<K::Err>,
{
    #[instrument(skip_all, fields(table = self.id_column))]
    pub async fn list_all(&self) -> Result<Vec<(K, Appraisal)>, StoreError> {
        let label = format!("appraisal_list_all:{}", self.id_column);
        let batches = execute_query(&self.table.query(), &label).await?;
        parse_keyed_appraisals(&batches, self.id_column, |s| {
            s.parse().map_err(StoreError::from)
        })
    }
}

impl AppraisalSource for SkeetStore {
    fn skeet_appraisals(&self) -> Appraisals<SkeetId> {
        Appraisals::new(self.skeet_appraisal_table.clone(), "skeet_id")
    }

    fn image_appraisals(&self) -> Appraisals<ImageId> {
        Appraisals::new(self.image_appraisal_table.clone(), "image_id")
    }
}

fn parse_single_appraisal(batches: &[RecordBatch]) -> Result<Option<Appraisal>, StoreError> {
    if batches.is_empty() || batches[0].num_rows() == 0 {
        return Ok(None);
    }
    let bands = typed_column::<StringArray>(&batches[0], "band")?;
    let appraisers = typed_column::<StringArray>(&batches[0], "appraiser")?;
    let band: Band = bands.value(0).parse()?;
    let appraiser: Appraiser = appraisers.value(0).parse()?;
    Ok(Some(Appraisal { band, appraiser }))
}

fn parse_keyed_appraisals<K>(
    batches: &[RecordBatch],
    id_column: &str,
    parse_id: impl Fn(&str) -> Result<K, StoreError>,
) -> Result<Vec<(K, Appraisal)>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let ids = typed_column::<StringArray>(batch, id_column)?;
        let bands = typed_column::<StringArray>(batch, "band")?;
        let appraisers = typed_column::<StringArray>(batch, "appraiser")?;
        for i in 0..batch.num_rows() {
            let id = parse_id(ids.value(i))?;
            let band: Band = bands.value(i).parse()?;
            let appraiser: Appraiser = appraisers.value(i).parse()?;
            results.push((id, Appraisal { band, appraiser }));
        }
    }
    Ok(results)
}
