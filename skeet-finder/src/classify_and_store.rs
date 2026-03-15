use face_detection::{
    ArchetypeConfig, Classification, ConfigVersion, FaceDetector, Quadrant, Rejection,
    annotate_image, classify,
};
use skeet_store::{
    Archetype, DiscoveredAt, ImageId, ImageRecord, OriginalAt, SkeetStore,
};
use tracing::{info, warn};

use crate::firehose::SkeetImage;

pub fn classify_image(
    skeet_image: SkeetImage,
    detector: &FaceDetector,
    text_detector: &text_detection::TextDetector,
    archetype_config: &ArchetypeConfig,
    config_version: &ConfigVersion,
) -> Result<ImageRecord, Vec<Rejection>> {
    let skin_mask = skin_detection::detect_skin(&skeet_image.image);
    let word_count = text_detector.count_characters(&skeet_image.image);
    let classification = classify(
        detector,
        &skeet_image.image,
        &skin_mask,
        word_count,
        archetype_config,
    );

    let quadrant = match classification {
        Classification::Accepted(q) => q,
        Classification::Rejected(reasons) => return Err(reasons),
    };

    let archetype = match quadrant {
        Quadrant::TopLeft => Archetype::TopLeft,
        Quadrant::TopRight => Archetype::TopRight,
        Quadrant::BottomLeft => Archetype::BottomLeft,
        Quadrant::BottomRight => Archetype::BottomRight,
    };

    let faces = detector.detect(&skeet_image.image);
    let face = faces
        .iter()
        .find(|f| f.is_frontal())
        .expect("classify accepted, so a frontal face exists");
    let annotated = annotate_image(&skeet_image.image, face, &skin_mask);

    Ok(ImageRecord {
        image_id: ImageId::new(),
        skeet_id: skeet_image.skeet_id,
        image: skeet_image.image,
        discovered_at: DiscoveredAt::now(),
        original_at: OriginalAt::new(skeet_image.original_at),
        archetype,
        annotated_image: annotated,
        config_version: config_version.clone(),
    })
}

pub async fn save(store: &SkeetStore, record: &ImageRecord, saved_count: &mut u64) {
    match store.add(record).await {
        Ok(()) => {
            *saved_count += 1;
            info!(
                saved = *saved_count,
                skeet_id = %record.skeet_id,
                archetype = %record.archetype,
                "saved image"
            );
        }
        Err(e) => {
            warn!(error = %e, "failed to save image to store");
        }
    }
}
