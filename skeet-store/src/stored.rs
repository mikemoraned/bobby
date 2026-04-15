use arrow_array::{LargeBinaryArray, RecordBatch, StringArray, TimestampMicrosecondArray};
use image::DynamicImage;
use shared::ModelVersion;
use tracing::instrument;

use crate::arrow_utils::{encode_image_as_png, micros_to_datetime, typed_column};
use crate::types::{DiscoveredAt, ImageId, ImageRecord, OriginalAt, SkeetId, Zone};
use crate::StoreError;

pub struct StoredImage {
    pub summary: StoredImageSummary,
    pub image: DynamicImage,
    pub annotated_image: DynamicImage,
}

impl StoredImage {
    pub fn content_matches(&self, other: &Self) -> Result<bool, StoreError> {
        let self_bytes = encode_image_as_png(&self.image)?;
        let other_bytes = encode_image_as_png(&other.image)?;
        Ok(self_bytes == other_bytes)
    }
}

impl From<StoredImage> for ImageRecord {
    fn from(stored: StoredImage) -> Self {
        Self {
            image_id: stored.summary.image_id,
            skeet_id: stored.summary.skeet_id,
            image: stored.image,
            discovered_at: stored.summary.discovered_at,
            original_at: stored.summary.original_at,
            zone: stored.summary.zone,
            annotated_image: stored.annotated_image,
            config_version: stored.summary.config_version,
            detected_text: stored.summary.detected_text,
        }
    }
}

#[derive(Clone)]
pub struct StoredImageSummary {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
    pub zone: Zone,
    pub config_version: ModelVersion,
    pub detected_text: String,
}

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
    #[instrument(skip(batch))]
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

    #[instrument(skip(self))]
    pub fn to_summary(&self, i: usize) -> Result<StoredImageSummary, StoreError> {
        let zone: Zone = self
            .archetypes
            .value(i)
            .parse()
            .map_err(|_| StoreError::InvalidZone(self.archetypes.value(i).to_string()))?;
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
pub fn batches_to_stored_images(
    batches: &[RecordBatch],
) -> Result<Vec<StoredImage>, StoreError> {
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
