use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchIterator, StringArray, TimestampMicrosecondArray};
use chrono::Utc;
use lancedb::query::QueryBase;
use shared::{Appraiser, Band};
use tracing::instrument;

use crate::arrow_utils::typed_column;
use crate::lancedb_utils::execute_query;
use crate::schema::{manual_image_appraisal_v1_schema, manual_skeet_appraisal_v1_schema};
use crate::types::{ImageId, SkeetId};
use crate::{SkeetStore, StoreError};

/// A stored manual appraisal: the band assigned and who assigned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appraisal {
    pub band: Band,
    pub appraiser: Appraiser,
}

impl SkeetStore {
    #[instrument(skip(self))]
    pub async fn set_skeet_band(
        &self,
        skeet_id: &SkeetId,
        band: Band,
        appraiser: &Appraiser,
    ) -> Result<(), StoreError> {
        self.skeet_appraisal_table
            .delete(&format!("skeet_id = '{skeet_id}'"))
            .await?;

        let schema = manual_skeet_appraisal_v1_schema();
        let skeet_id_str = skeet_id.to_string();
        let band_str = band.to_string();
        let appraiser_str = appraiser.to_string();
        let now_us = Utc::now().timestamp_micros();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![skeet_id_str.as_str()])),
                Arc::new(StringArray::from(vec![band_str.as_str()])),
                Arc::new(StringArray::from(vec![appraiser_str.as_str()])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![now_us]).with_timezone("UTC"),
                ),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.skeet_appraisal_table.add(batches).execute().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn clear_skeet_band(&self, skeet_id: &SkeetId) -> Result<(), StoreError> {
        self.skeet_appraisal_table
            .delete(&format!("skeet_id = '{skeet_id}'"))
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_skeet_band(
        &self,
        skeet_id: &SkeetId,
    ) -> Result<Option<Appraisal>, StoreError> {
        let query = self
            .skeet_appraisal_table
            .query()
            .only_if(format!("skeet_id = '{skeet_id}'"))
            .limit(1);
        let batches = execute_query(&query, "get_skeet_band").await?;
        parse_single_appraisal(&batches)
    }

    #[instrument(skip(self))]
    pub async fn list_all_skeet_appraisals(
        &self,
    ) -> Result<Vec<(SkeetId, Appraisal)>, StoreError> {
        let batches =
            execute_query(&self.skeet_appraisal_table.query(), "list_all_skeet_appraisals").await?;
        parse_keyed_appraisals(&batches, "skeet_id", |s| s.parse().map_err(StoreError::from))
    }

    #[instrument(skip(self))]
    pub async fn set_image_band(
        &self,
        image_id: &ImageId,
        band: Band,
        appraiser: &Appraiser,
    ) -> Result<(), StoreError> {
        self.image_appraisal_table
            .delete(&format!("image_id = '{image_id}'"))
            .await?;

        let schema = manual_image_appraisal_v1_schema();
        let image_id_str = image_id.to_string();
        let band_str = band.to_string();
        let appraiser_str = appraiser.to_string();
        let now_us = Utc::now().timestamp_micros();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![image_id_str.as_str()])),
                Arc::new(StringArray::from(vec![band_str.as_str()])),
                Arc::new(StringArray::from(vec![appraiser_str.as_str()])),
                Arc::new(
                    TimestampMicrosecondArray::from(vec![now_us]).with_timezone("UTC"),
                ),
            ],
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.image_appraisal_table.add(batches).execute().await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn clear_image_band(&self, image_id: &ImageId) -> Result<(), StoreError> {
        self.image_appraisal_table
            .delete(&format!("image_id = '{image_id}'"))
            .await?;
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_image_band(
        &self,
        image_id: &ImageId,
    ) -> Result<Option<Appraisal>, StoreError> {
        let query = self
            .image_appraisal_table
            .query()
            .only_if(format!("image_id = '{image_id}'"))
            .limit(1);
        let batches = execute_query(&query, "get_image_band").await?;
        parse_single_appraisal(&batches)
    }

    #[instrument(skip(self))]
    pub async fn list_all_image_appraisals(
        &self,
    ) -> Result<Vec<(ImageId, Appraisal)>, StoreError> {
        let batches =
            execute_query(&self.image_appraisal_table.query(), "list_all_image_appraisals").await?;
        parse_keyed_appraisals(&batches, "image_id", |s| s.parse().map_err(StoreError::from))
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
