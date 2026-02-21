use image::DynamicImage;

const THUMBNAIL_SIZE: u32 = 64;
const SKIN_THRESHOLD: f64 = 0.40;

/// Detects images with excessive skin exposure using YCbCr color space.
/// Downscales to 64x64, converts each pixel to YCbCr, and classifies pixels
/// as "skin" when Cb ∈ [77, 127] and Cr ∈ [133, 173]. These chrominance
/// ranges cluster tightly around skin tones regardless of luminance, working
/// across skin tones and lighting conditions. If >= 40% of pixels are skin,
/// the image is filtered out.
pub fn is_excessive_skin(img: &DynamicImage) -> bool {
    let thumb = img.resize_exact(
        THUMBNAIL_SIZE,
        THUMBNAIL_SIZE,
        image::imageops::FilterType::Nearest,
    );
    let rgb = thumb.to_rgb8();
    let total_pixels = (THUMBNAIL_SIZE * THUMBNAIL_SIZE) as usize;

    let skin_pixels = rgb
        .pixels()
        .filter(|pixel| {
            let (_, cb, cr) = rgb_to_ycbcr(pixel[0], pixel[1], pixel[2]);
            (77..=127).contains(&cb) && (133..=173).contains(&cr)
        })
        .count();

    let fraction = skin_pixels as f64 / total_pixels as f64;
    fraction >= SKIN_THRESHOLD
}

fn rgb_to_ycbcr(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r = r as f64;
    let g = g as f64;
    let b = b as f64;
    let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
    let cb = (128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b).clamp(0.0, 255.0) as u8;
    let cr = (128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b).clamp(0.0, 255.0) as u8;
    (y, cb, cr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn solid_image(r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, Rgb([r, g, b])))
    }

    #[test]
    fn all_skin_tone_is_filtered() {
        // Warm beige — falls squarely in skin Cb/Cr range
        let img = solid_image(200, 150, 120);
        assert!(is_excessive_skin(&img));
    }

    #[test]
    fn mostly_skin_with_some_nonskin_is_filtered() {
        let mut buf = RgbImage::new(100, 100);
        for (x, _y, pixel) in buf.enumerate_pixels_mut() {
            *pixel = if x < 55 {
                // non-skin: pure blue
                Rgb([0, 0, 255])
            } else {
                // skin tone
                Rgb([200, 150, 120])
            };
        }
        // 45% skin > 40% threshold
        assert!(is_excessive_skin(&DynamicImage::ImageRgb8(buf)));
    }

    #[test]
    fn small_skin_patch_not_filtered() {
        let mut buf = RgbImage::new(100, 100);
        for (x, _y, pixel) in buf.enumerate_pixels_mut() {
            *pixel = if x < 75 {
                Rgb([0, 0, 255])
            } else {
                Rgb([200, 150, 120])
            };
        }
        // 25% skin < 40% threshold
        assert!(!is_excessive_skin(&DynamicImage::ImageRgb8(buf)));
    }

    #[test]
    fn all_blue_not_filtered() {
        let img = solid_image(0, 0, 255);
        assert!(!is_excessive_skin(&img));
    }

    #[test]
    fn all_green_not_filtered() {
        let img = solid_image(0, 255, 0);
        assert!(!is_excessive_skin(&img));
    }
}
