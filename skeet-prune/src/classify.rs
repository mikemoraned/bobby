use face_detection::{Face, FaceDetector, annotate_image};
use image::{DynamicImage, GrayImage};
use shared::{
    Classification, ModelVersion, Percentage, PruneConfig, Rejection,
    SkeetImage, Zone,
};
use skeet_store::{DiscoveredAt, ImageId, ImageRecord, OriginalAt};

/// Classify an image: given pre-detected faces, check area and skin thresholds,
/// return quadrant or rejection.
pub fn classify(
    faces: &[Face],
    image: &DynamicImage,
    skin_mask: &GrayImage,
    config: &PruneConfig,
) -> Classification {

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

    if !reasons.is_empty() {
        return Classification::Rejected(reasons);
    }

    let zone = face_detection::face_zone(face, image.width(), image.height());
    if is_accepted_zone(zone) {
        Classification::Accepted(zone)
    } else {
        Classification::Rejected(vec![Rejection::FaceNotInAcceptedZone])
    }
}

const fn is_accepted_zone(zone: Zone) -> bool {
    matches!(
        zone,
        Zone::TopLeft
            | Zone::TopRight
            | Zone::CenterLeft
            | Zone::CenterRight
            | Zone::BottomLeft
            | Zone::BottomRight
    )
}

pub fn classify_image(
    skeet_image: SkeetImage,
    detector: &FaceDetector,
    prune_config: &PruneConfig,
    config_version: &ModelVersion,
) -> Result<ImageRecord, Vec<Rejection>> {
    let skin_mask = skin_detection::detect_skin(&skeet_image.image);
    let faces = detector.detect(&skeet_image.image);
    let classification = classify(
        &faces,
        &skeet_image.image,
        &skin_mask,
        prune_config,
    );

    let zone = match classification {
        Classification::Accepted(z) => z,
        Classification::Rejected(reasons) => return Err(reasons),
    };

    let face = faces
        .iter()
        .find(|f| f.is_frontal())
        .expect("classify accepted, so a frontal face exists");

    let annotated = annotate_image(&skeet_image.image, face, &skin_mask);

    Ok(ImageRecord {
        image_id: ImageId::from_image(&skeet_image.image),
        skeet_id: skeet_image.skeet_id,
        image: skeet_image.image,
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(skeet_image.original_at),
        zone,
        annotated_image: annotated,
        config_version: config_version.clone(),
        detected_text: String::new(),
    })
}
