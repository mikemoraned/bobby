use std::io::Cursor;

use arrow_array::{Array, RecordBatch, TimestampMicrosecondArray};
use chrono::{DateTime, TimeZone, Utc};
use image::DynamicImage;
use tracing::instrument;

use crate::StoreError;

#[instrument(skip(batch))]
pub fn typed_column<'a, T: Array + 'static>(
    batch: &'a RecordBatch,
    name: &str,
) -> Result<&'a T, StoreError> {
    batch
        .column_by_name(name)
        .and_then(|col| col.as_any().downcast_ref::<T>())
        .ok_or_else(|| StoreError::ColumnTypeMismatch {
            column: name.to_string(),
        })
}

#[instrument(skip(img))]
pub fn encode_image_as_png(img: &DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

pub fn micros_to_datetime(micros: i64) -> DateTime<Utc> {
    Utc.timestamp_micros(micros)
        .single()
        .expect("valid timestamp from store")
}

pub type DateTimeRange = (DateTime<Utc>, DateTime<Utc>);

pub fn min_max_timestamp(
    batches: &[RecordBatch],
    column: &str,
) -> Result<Option<DateTimeRange>, StoreError> {
    let mut overall_min: Option<i64> = None;
    let mut overall_max: Option<i64> = None;
    for batch in batches {
        let col = typed_column::<TimestampMicrosecondArray>(batch, column)?;
        if let Some(batch_min) = arrow_arith::aggregate::min(col) {
            overall_min = Some(match overall_min {
                Some(prev) if prev <= batch_min => prev,
                _ => batch_min,
            });
        }
        if let Some(batch_max) = arrow_arith::aggregate::max(col) {
            overall_max = Some(match overall_max {
                Some(prev) if prev >= batch_max => prev,
                _ => batch_max,
            });
        }
    }
    Ok(overall_min
        .zip(overall_max)
        .map(|(min, max)| (micros_to_datetime(min), micros_to_datetime(max))))
}
