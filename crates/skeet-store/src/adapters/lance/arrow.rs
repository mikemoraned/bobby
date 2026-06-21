use std::io::Cursor;

use crate::StoreError;
use arrow_array::{Array, RecordBatch};
use chrono::{DateTime, TimeZone, Utc};
use image::DynamicImage;

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

pub fn encode_image_as_png(img: &DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

// Microseconds read back from the store are always in chrono's representable range
// and Utc is never ambiguous, so `single()` is always `Some`.
#[allow(clippy::expect_used)]
pub fn micros_to_datetime(micros: i64) -> DateTime<Utc> {
    Utc.timestamp_micros(micros)
        .single()
        .expect("valid timestamp from store")
}
