use arrow_array::{LargeBinaryArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use shared::{DiscoveredAt, ModelVersion, OriginalAt, SkeetId, Zone};
use tracing::instrument;

use super::arrow::{micros_to_datetime, typed_column};
use crate::{StoreError, StoredImage, StoredImageSummary, StoredOriginal};

pub struct SummaryColumns<'a> {
    image_ids: &'a StringArray,
    skeet_ids: &'a StringArray,
    discovered_ats: &'a TimestampMicrosecondArray,
    original_ats: &'a TimestampMicrosecondArray,
    archetypes: &'a StringArray,
    config_versions: &'a StringArray,
    detected_texts: &'a StringArray,
}

impl<'a> SummaryColumns<'a> {
    pub fn extract(batch: &'a RecordBatch) -> Result<Self, StoreError> {
        Ok(Self {
            image_ids: typed_column::<StringArray>(batch, "image_id")?,
            skeet_ids: typed_column::<StringArray>(batch, "skeet_id")?,
            discovered_ats: typed_column::<TimestampMicrosecondArray>(batch, "discovered_at")?,
            original_ats: typed_column::<TimestampMicrosecondArray>(batch, "original_at")?,
            archetypes: typed_column::<StringArray>(batch, "archetype")?,
            config_versions: typed_column::<StringArray>(batch, "config_version")?,
            detected_texts: typed_column::<StringArray>(batch, "detected_text")?,
        })
    }

    pub fn to_summary(&self, i: usize) -> Result<StoredImageSummary, StoreError> {
        let zone: Zone = self.archetypes.value(i).parse()?;
        let config_version = ModelVersion::from(self.config_versions.value(i));

        Ok(StoredImageSummary {
            image_id: self.image_ids.value(i).parse()?,
            skeet_id: SkeetId::new(self.skeet_ids.value(i))?,
            discovered_at: DiscoveredAt::new(micros_to_datetime(self.discovered_ats.value(i))),
            original_at: OriginalAt::new(micros_to_datetime(self.original_ats.value(i))),
            zone,
            config_version,
            detected_text: self.detected_texts.value(i).to_string(),
        })
    }
}

#[instrument(skip(batches))]
pub fn batches_to_summaries(
    batches: &[RecordBatch],
) -> Result<Vec<StoredImageSummary>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let cols = SummaryColumns::extract(batch)?;
        for i in 0..batch.num_rows() {
            results.push(cols.to_summary(i)?);
        }
    }
    Ok(results)
}

#[instrument(skip(batches))]
pub fn batches_to_stored_images(batches: &[RecordBatch]) -> Result<Vec<StoredImage>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let cols = SummaryColumns::extract(batch)?;
        let images = typed_column::<LargeBinaryArray>(batch, "image")?;
        let annotated_images = typed_column::<LargeBinaryArray>(batch, "annotated_image")?;

        for i in 0..batch.num_rows() {
            let summary = cols.to_summary(i)?;
            let image = image::load_from_memory(images.value(i))?;
            let annotated_image = image::load_from_memory(annotated_images.value(i))?;
            results.push(StoredImage {
                summary,
                image,
                annotated_image,
            });
        }
    }
    Ok(results)
}

#[instrument(skip(batches))]
pub fn batches_to_original_images(
    batches: &[RecordBatch],
) -> Result<Vec<StoredOriginal>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let cols = SummaryColumns::extract(batch)?;
        let images = typed_column::<LargeBinaryArray>(batch, "image")?;
        for i in 0..batch.num_rows() {
            let summary = cols.to_summary(i)?;
            let image = image::load_from_memory(images.value(i))?;
            results.push(StoredOriginal { summary, image });
        }
    }
    Ok(results)
}
