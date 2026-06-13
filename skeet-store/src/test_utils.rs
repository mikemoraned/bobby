// Test scaffolding: every fallible call here is a fixture setup step whose
// failure should abort the test loudly, so panicking via `expect` is the
// intended behaviour. (The module isn't `#[cfg(test)]` — it's exposed through the
// `test-helpers` feature — so the in-tests clippy allowance doesn't reach it.)
#![allow(clippy::expect_used)]

use chrono::Utc;
use image::{DynamicImage, ImageBuffer, Rgba};

use shared::ImageId;

use crate::{DiscoveredAt, ImageRecord, ModelVersion, OriginalAt, SkeetStore, Zone};

pub fn test_image() -> DynamicImage {
    test_image_with_color(255, 0, 0)
}

pub fn test_image_with_color(r: u8, g: u8, b: u8) -> DynamicImage {
    DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([r, g, b, 255])))
}

pub fn make_record(suffix: &str, r: u8, g: u8, b: u8) -> ImageRecord {
    make_record_at(suffix, r, g, b, DiscoveredAt::now())
}

pub fn make_record_at(
    suffix: &str,
    r: u8,
    g: u8,
    b: u8,
    discovered_at: DiscoveredAt,
) -> ImageRecord {
    let img = test_image_with_color(r, g, b);
    ImageRecord {
        image_id: ImageId::from_image(&img),
        skeet_id: format!("at://did:plc:abc/app.bsky.feed.post/{suffix}")
            .parse()
            .expect("valid AT URI"),
        image: img,
        discovered_at,
        original_at: OriginalAt::new(Utc::now()),
        zone: Zone::TopRight,
        annotated_image: test_image(),
        config_version: ModelVersion::from("test"),
        detected_text: String::new(),
    }
}

pub async fn open_temp_store(dir: &tempfile::TempDir) -> SkeetStore {
    SkeetStore::open(dir.path().to_str().expect("valid path"), vec![], "test")
        .await
        .expect("open store")
}
