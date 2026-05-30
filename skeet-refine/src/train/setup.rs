use std::collections::HashMap;

use shared::{Band, ImageId};

use crate::train::TrainError;

pub fn label_train_items(
    train: &[ImageId],
    band_by_id: &HashMap<ImageId, Band>,
) -> Result<Vec<(ImageId, Band)>, TrainError> {
    train
        .iter()
        .map(|id| {
            let band = band_by_id
                .get(id)
                .copied()
                .ok_or_else(|| TrainError::AppraisalMissing(id.to_string()))?;
            Ok((id.clone(), band))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use test_support::marker_image;

    use super::*;

    #[test]
    fn label_train_items_pairs_ids_with_their_bands() {
        let id_a = ImageId::from_image(&marker_image(0));
        let id_b = ImageId::from_image(&marker_image(1));
        let mut band_by_id = HashMap::new();
        band_by_id.insert(id_a.clone(), Band::HighQuality);
        band_by_id.insert(id_b.clone(), Band::Low);

        let labelled = label_train_items(&[id_a.clone(), id_b.clone()], &band_by_id).expect("ok");

        assert_eq!(labelled.len(), 2);
        assert_eq!(labelled[0], (id_a, Band::HighQuality));
        assert_eq!(labelled[1], (id_b, Band::Low));
    }

    #[test]
    fn label_train_items_errors_when_appraisal_missing() {
        let id = ImageId::from_image(&marker_image(0));
        let band_by_id: HashMap<ImageId, Band> = HashMap::new();

        let err = label_train_items(&[id.clone()], &band_by_id).expect_err("should fail");
        assert!(matches!(err, TrainError::AppraisalMissing(s) if s == id.to_string()));
    }
}
