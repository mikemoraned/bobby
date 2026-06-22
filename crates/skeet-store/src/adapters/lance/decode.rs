use arrow_array::{
    Float32Array, LargeBinaryArray, RecordBatch, StringArray, TimestampMicrosecondArray,
};
use shared::{DiscoveredAt, ImageId, ModelVersion, OriginalAt, SkeetId, Zone};
use tracing::instrument;

use super::arrow::{micros_to_datetime, typed_column};
use crate::{Score, StoreError, StoredImage, StoredImageSummary, StoredOriginal};

/// Typed columns of the scores table: `(image_id, score, model_version)`. Shared
/// by every scan that builds a [`ScoresMap`](crate::model::ScoresMap) entry.
type ScoreColumns<'a> = (&'a StringArray, &'a Float32Array, &'a StringArray);

pub fn score_columns(batch: &RecordBatch) -> Result<ScoreColumns<'_>, StoreError> {
    Ok((
        typed_column::<StringArray>(batch, "image_id")?,
        typed_column::<Float32Array>(batch, "score")?,
        typed_column::<StringArray>(batch, "model_version")?,
    ))
}

pub fn decode_score_row(
    (ids, scores, model_versions): &ScoreColumns<'_>,
    i: usize,
) -> Result<(ImageId, (Score, ModelVersion)), StoreError> {
    let image_id: ImageId = ids.value(i).parse()?;
    let score = Score::new(scores.value(i))?;
    let model_version = ModelVersion::from(model_versions.value(i));
    Ok((image_id, (score, model_version)))
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

/// Decode every row of `batches` into a `T`, extracting the typed columns once
/// per batch.
///
/// `extract` names the columns it needs (returning whatever bundle of typed
/// arrays the decode requires); `decode` builds one value from row `i` of that
/// bundle. Collects into a `Vec`; callers wanting a set or map collect again
/// from it. Co-locating the column list with the per-row builder keeps the two
/// in step and removes the otherwise-repeated batch/row iteration.
pub fn decode_rows<'a, C, T>(
    batches: &'a [RecordBatch],
    extract: impl Fn(&'a RecordBatch) -> Result<C, StoreError>,
    decode: impl Fn(&C, usize) -> Result<T, StoreError>,
) -> Result<Vec<T>, StoreError> {
    let mut results = Vec::new();
    for batch in batches {
        let cols = extract(batch)?;
        for i in 0..batch.num_rows() {
            results.push(decode(&cols, i)?);
        }
    }
    Ok(results)
}

#[instrument(skip(batches))]
pub fn batches_to_summaries(
    batches: &[RecordBatch],
) -> Result<Vec<StoredImageSummary>, StoreError> {
    decode_rows(batches, SummaryColumns::extract, |cols, i| {
        cols.to_summary(i)
    })
}

#[instrument(skip(batches))]
pub fn batches_to_stored_images(batches: &[RecordBatch]) -> Result<Vec<StoredImage>, StoreError> {
    decode_rows(
        batches,
        |batch| {
            Ok((
                SummaryColumns::extract(batch)?,
                typed_column::<LargeBinaryArray>(batch, "image")?,
                typed_column::<LargeBinaryArray>(batch, "annotated_image")?,
            ))
        },
        |(cols, images, annotated_images), i| {
            Ok(StoredImage {
                summary: cols.to_summary(i)?,
                image: image::load_from_memory(images.value(i))?,
                annotated_image: image::load_from_memory(annotated_images.value(i))?,
            })
        },
    )
}

#[instrument(skip(batches))]
pub fn batches_to_original_images(
    batches: &[RecordBatch],
) -> Result<Vec<StoredOriginal>, StoreError> {
    decode_rows(
        batches,
        |batch| {
            Ok((
                SummaryColumns::extract(batch)?,
                typed_column::<LargeBinaryArray>(batch, "image")?,
            ))
        },
        |(cols, images), i| {
            Ok(StoredOriginal {
                summary: cols.to_summary(i)?,
                image: image::load_from_memory(images.value(i))?,
            })
        },
    )
}
