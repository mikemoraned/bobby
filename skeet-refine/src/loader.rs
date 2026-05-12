use std::collections::HashMap;
use std::str::FromStr;

use image::DynamicImage;
use shared::Band;
use skeet_store::{ImageId, SkeetStore, StoreError};

/// An image fetched from the store paired with its appraised `Band`. The binary
/// label for the refine classifier is `band.is_visible_in_feed()`.
#[derive(Debug)]
pub struct LabelledImage {
    pub id: ImageId,
    pub image: DynamicImage,
    pub band: Band,
}

impl LabelledImage {
    pub const fn is_positive(&self) -> bool {
        self.band.is_visible_in_feed()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoaderError {
    #[error("invalid image id in split: {0}")]
    InvalidImageId(String),
    #[error("image id {0} is no longer present in the store appraisals")]
    AppraisalMissing(String),
    #[error("image id {0} is no longer present in the store images table")]
    ImageMissing(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Load every image appraisal from the store and index it by `ImageId`. A
/// single index can then be used to label any subset of images for downstream
/// scoring runs.
pub async fn load_band_index(store: &SkeetStore) -> Result<HashMap<ImageId, Band>, LoaderError> {
    let appraisals = store.list_all_image_appraisals().await?;
    Ok(appraisals.into_iter().map(|(id, a)| (id, a.band)).collect())
}

/// Resolve `image_id_strs` into in-memory `LabelledImage`s. Errors if any id is
/// malformed, missing from the appraisal index, or absent from the images table.
pub async fn load_labelled_images(
    store: &SkeetStore,
    band_by_id: &HashMap<ImageId, Band>,
    image_id_strs: &[String],
) -> Result<Vec<LabelledImage>, LoaderError> {
    let ids: Vec<ImageId> = image_id_strs
        .iter()
        .map(|s| ImageId::from_str(s).map_err(|_| LoaderError::InvalidImageId(s.clone())))
        .collect::<Result<_, _>>()?;

    let bands: Vec<Band> = ids
        .iter()
        .map(|id| {
            band_by_id
                .get(id)
                .copied()
                .ok_or_else(|| LoaderError::AppraisalMissing(id.to_string()))
        })
        .collect::<Result<_, _>>()?;

    let originals = store.get_originals_by_ids(&ids).await?;
    let mut images_by_id: HashMap<ImageId, DynamicImage> = originals
        .into_iter()
        .map(|o| (o.summary.image_id, o.image))
        .collect();

    ids.into_iter()
        .zip(bands)
        .map(|(id, band)| {
            let image = images_by_id
                .remove(&id)
                .ok_or_else(|| LoaderError::ImageMissing(id.to_string()))?;
            Ok(LabelledImage { id, image, band })
        })
        .collect()
}
