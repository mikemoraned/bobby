use image::DynamicImage;

const THUMBNAIL_SIZE: u32 = 64;
const QUANTIZE_DIVISOR: u8 = 8;
const TOP_COLORS: usize = 5;
const TEXT_THRESHOLD: f64 = 0.75;

/// Detects text-heavy images (screenshots, text memes) using color concentration.
/// Downscales to 64x64, quantizes colors to 32 levels per channel, then checks
/// whether the top 5 most frequent colors cover >= 75% of all pixels. Natural
/// photos have rich color distributions; text-heavy images are dominated by a
/// few colors (background + font colors).
pub fn is_mostly_text(img: &DynamicImage) -> bool {
    let thumb = img.resize_exact(
        THUMBNAIL_SIZE,
        THUMBNAIL_SIZE,
        image::imageops::FilterType::Nearest,
    );
    let rgb = thumb.to_rgb8();
    let total_pixels = (THUMBNAIL_SIZE * THUMBNAIL_SIZE) as usize;

    let mut histogram = std::collections::HashMap::new();
    for pixel in rgb.pixels() {
        let key = (
            pixel[0] / QUANTIZE_DIVISOR,
            pixel[1] / QUANTIZE_DIVISOR,
            pixel[2] / QUANTIZE_DIVISOR,
        );
        *histogram.entry(key).or_insert(0u32) += 1;
    }

    let mut counts: Vec<u32> = histogram.into_values().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));

    let top_sum: u32 = counts.iter().take(TOP_COLORS).sum();
    let fraction = top_sum as f64 / total_pixels as f64;

    fraction >= TEXT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn solid_color_is_text() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, Rgb([255, 255, 255])));
        assert!(is_mostly_text(&img));
    }

    #[test]
    fn two_tone_is_text() {
        let mut buf = RgbImage::new(100, 100);
        for (x, _y, pixel) in buf.enumerate_pixels_mut() {
            *pixel = if x < 50 {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            };
        }
        assert!(is_mostly_text(&DynamicImage::ImageRgb8(buf)));
    }

    #[test]
    fn random_noise_is_not_text() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut buf = RgbImage::new(100, 100);
        for (x, y, pixel) in buf.enumerate_pixels_mut() {
            let mut h = DefaultHasher::new();
            (x, y).hash(&mut h);
            let hash = h.finish();
            let bytes = hash.to_le_bytes();
            *pixel = Rgb([bytes[0], bytes[1], bytes[2]]);
        }
        assert!(!is_mostly_text(&DynamicImage::ImageRgb8(buf)));
    }

    #[test]
    fn gradient_is_not_text() {
        let mut buf = RgbImage::new(256, 256);
        for (x, y, pixel) in buf.enumerate_pixels_mut() {
            *pixel = Rgb([x as u8, y as u8, ((x + y) / 2) as u8]);
        }
        assert!(!is_mostly_text(&DynamicImage::ImageRgb8(buf)));
    }
}
