use face_detection::{FaceDetector, TextRegion, annotate_image};
use image::{DynamicImage, GrayImage};
use shared::{
    ArchetypeConfig, Classification, ConfigVersion, Percentage, Rejection,
    SkeetImage, Zone,
};
use skeet_store::{DiscoveredAt, ImageId, ImageRecord, OriginalAt};

/// Classify an image: detect frontal faces, check area, skin, and text thresholds,
/// return quadrant or rejection.
pub fn classify(
    detector: &FaceDetector,
    image: &DynamicImage,
    skin_mask: &GrayImage,
    text_area_pct: Percentage,
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

    if text_area_pct > config.max_text_area_pct {
        reasons.push(Rejection::TooMuchText);
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
    text_detector: &text_detection::TextDetector,
    archetype_config: &ArchetypeConfig,
    config_version: &ConfigVersion,
) -> Result<ImageRecord, Vec<Rejection>> {
    let skin_mask = skin_detection::detect_skin(&skeet_image.image);
    let text_result = text_detector.detect(&skeet_image.image);
    let text_area_pct = Percentage::new(
        text_result.text_area_pct(skeet_image.image.width(), skeet_image.image.height()),
    );
    let classification = classify(
        detector,
        &skeet_image.image,
        &skin_mask,
        text_area_pct,
        archetype_config,
    );

    let zone = match classification {
        Classification::Accepted(z) => z,
        Classification::Rejected(reasons) => return Err(reasons),
    };

    let faces = detector.detect(&skeet_image.image);
    let face = faces
        .iter()
        .find(|f| f.is_frontal())
        .expect("classify accepted, so a frontal face exists");

    let text_regions: Vec<TextRegion> = text_result
        .lines
        .iter()
        .map(|line| TextRegion {
            x: line.x,
            y: line.y,
            width: line.width,
            height: line.height,
        })
        .collect();
    let annotated = annotate_image(&skeet_image.image, face, &skin_mask, &text_regions);
    let detected_text = text_result.full_text();

    Ok(ImageRecord {
        image_id: ImageId::new(),
        skeet_id: skeet_image.skeet_id,
        image: skeet_image.image,
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(skeet_image.original_at),
        zone,
        annotated_image: annotated,
        config_version: config_version.clone(),
        detected_text,
    })
}
