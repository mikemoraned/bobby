use image::DynamicImage;
use shared::{DiscoveredAt, ImageId, ModelVersion, OriginalAt, SkeetId, Zone};

#[derive(Debug, Clone)]
pub struct ImageRecord {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub image: DynamicImage,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
    pub zone: Zone,
    pub annotated_image: DynamicImage,
    pub config_version: ModelVersion,
    pub detected_text: String,
}

pub struct StoredImage {
    pub summary: StoredImageSummary,
    pub image: DynamicImage,
    pub annotated_image: DynamicImage,
}

/// A fetched image without the annotated overlay — used by callers that only
/// need the original pixels (e.g. live-refine scoring).
pub struct StoredOriginal {
    pub summary: StoredImageSummary,
    pub image: DynamicImage,
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
