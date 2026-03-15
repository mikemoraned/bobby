use image::{DynamicImage, GrayImage};
use shared::{
    ArchetypeConfig, Classification, Percentage, Rejection, Zone,
};

/// Classify an image: detect frontal faces, check area, skin, and text thresholds,
/// return quadrant or rejection.
pub fn classify(
    detector: &face_detection::FaceDetector,
    image: &DynamicImage,
    skin_mask: &GrayImage,
    word_count: usize,
    config: &ArchetypeConfig,
) -> Classification {
    let faces = detector.detect(image);

    if faces.len() > 1 {
        return Classification::Rejected(vec![Rejection::TooManyFaces]);
    }

    let Some(face) = faces.iter().find(|f| f.is_frontal()) else {
        return Classification::Rejected(vec![Rejection::TooFewFrontalFaces]);
    };

    let pct = face.area_pct(image.width(), image.height());
    let mut reasons = Vec::new();

    if pct < config.min_face_area_pct {
        reasons.push(Rejection::FaceTooSmall);
    }
    if pct > config.max_face_area_pct {
        reasons.push(Rejection::FaceTooLarge);
    }

    // Skin detection checks
    let face_skin = skin_detection::skin_pct_in_rect(
        skin_mask,
        face.x as u32,
        face.y as u32,
        face.width as u32,
        face.height as u32,
    );
    let outside_skin = skin_detection::skin_pct_outside_rect(
        skin_mask,
        face.x as u32,
        face.y as u32,
        face.width as u32,
        face.height as u32,
    );

    if Percentage::new(face_skin) < config.min_face_skin_pct {
        reasons.push(Rejection::TooLittleFaceSkin);
    }
    if Percentage::new(outside_skin) > config.max_outside_face_skin_pct {
        reasons.push(Rejection::TooMuchSkinOutsideFace);
    }

    if word_count > config.max_glyphs_allowed as usize {
        reasons.push(Rejection::TooMuchText);
    }

    if !reasons.is_empty() {
        return Classification::Rejected(reasons);
    }

    match face_detection::face_zone(face, image.width(), image.height()) {
        Zone::Quarter(quadrant) => Classification::Accepted(quadrant),
        Zone::Central => Classification::Rejected(vec![Rejection::FaceInCentralZone]),
    }
}
