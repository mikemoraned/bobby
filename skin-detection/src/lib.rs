#![warn(clippy::all, clippy::nursery)]

use image::{DynamicImage, GrayImage, Luma};

/// Classify each pixel as skin (255) or not-skin (0) using combined
/// RGB + YCbCr rules (Kovac, Peer & Solina, 2003).
///
/// This approach works across skin tones because the YCbCr chrominance
/// channels (Cb, Cr) cluster tightly for skin regardless of luminance,
/// while the RGB rules filter out common false positives (wood, sand, brick).
pub fn detect_skin(image: &DynamicImage) -> GrayImage {
    let rgb = image.to_rgb8();
    let (w, h) = rgb.dimensions();
    let mut mask = GrayImage::new(w, h);

    for (x, y, pixel) in rgb.enumerate_pixels() {
        let r = f32::from(pixel[0]);
        let g = f32::from(pixel[1]);
        let b = f32::from(pixel[2]);

        let is_skin = is_skin_pixel(r, g, b);
        mask.put_pixel(x, y, Luma([if is_skin { 255 } else { 0 }]));
    }

    mask
}

/// Combined RGB + YCbCr skin pixel test.
fn is_skin_pixel(r: f32, g: f32, b: f32) -> bool {
    // RGB rules (Kovac/Peer/Solina)
    if r <= 95.0 || g <= 40.0 || b <= 20.0 {
        return false;
    }

    let max_rgb = r.max(g).max(b);
    let min_rgb = r.min(g).min(b);
    if max_rgb - min_rgb <= 15.0 {
        return false;
    }

    if (r - g).abs() <= 15.0 {
        return false;
    }

    if r <= g || r <= b {
        return false;
    }

    // YCbCr rules
    let cb = (-0.169f32).mul_add(r, (-0.331f32).mul_add(g, 0.500f32.mul_add(b, 128.0)));
    let cr = 0.500f32.mul_add(r, (-0.419f32).mul_add(g, (-0.081f32).mul_add(b, 128.0)));

    (77.0..=127.0).contains(&cb) && (133.0..=173.0).contains(&cr)
}

/// Compute the percentage of skin pixels within a rectangular region.
pub fn skin_pct_in_rect(mask: &GrayImage, x: u32, y: u32, w: u32, h: u32) -> f32 {
    let rect = clamp_rect(mask, x, y, w, h);
    skin_pct(mask, |px, py| rect.contains(px, py))
}

/// Compute the percentage of skin pixels outside a rectangular region.
pub fn skin_pct_outside_rect(mask: &GrayImage, x: u32, y: u32, w: u32, h: u32) -> f32 {
    let rect = clamp_rect(mask, x, y, w, h);
    skin_pct(mask, |px, py| !rect.contains(px, py))
}

struct Rect {
    x_start: u32,
    x_end: u32,
    y_start: u32,
    y_end: u32,
}

impl Rect {
    const fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x_start && px < self.x_end && py >= self.y_start && py < self.y_end
    }
}

fn clamp_rect(mask: &GrayImage, x: u32, y: u32, w: u32, h: u32) -> Rect {
    let img_w = mask.width();
    let img_h = mask.height();
    Rect {
        x_start: x.min(img_w),
        x_end: (x + w).min(img_w),
        y_start: y.min(img_h),
        y_end: (y + h).min(img_h),
    }
}

fn skin_pct(mask: &GrayImage, include: impl Fn(u32, u32) -> bool) -> f32 {
    let mut total = 0u32;
    let mut skin = 0u32;

    for py in 0..mask.height() {
        for px in 0..mask.width() {
            if include(px, py) {
                total += 1;
                if mask.get_pixel(px, py).0[0] > 0 {
                    skin += 1;
                }
            }
        }
    }

    if total == 0 {
        return 0.0;
    }

    (skin as f32 / total as f32) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_red_is_not_skin() {
        // Pure red (255, 0, 0) fails G > 40 and B > 20
        assert!(!is_skin_pixel(255.0, 0.0, 0.0));
    }

    #[test]
    fn typical_light_skin_is_detected() {
        // A typical light skin tone
        assert!(is_skin_pixel(200.0, 150.0, 120.0));
    }

    #[test]
    fn blue_is_not_skin() {
        assert!(!is_skin_pixel(50.0, 50.0, 200.0));
    }

    #[test]
    fn gray_is_not_skin() {
        // Gray fails max-min > 15 and |R-G| > 15
        assert!(!is_skin_pixel(128.0, 128.0, 128.0));
    }

    // ─── detect_skin ───────────────────────────────────────────

    #[test]
    fn detect_skin_dimensions_match_input() {
        let img = DynamicImage::new_rgb8(32, 24);
        let mask = detect_skin(&img);
        assert_eq!(mask.dimensions(), (32, 24));
    }

    #[test]
    fn detect_skin_marks_skin_colored_pixels() {
        let mut img = image::RgbImage::new(4, 4);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([200, 150, 120]);
        }
        let mask = detect_skin(&DynamicImage::ImageRgb8(img));
        assert!(mask.pixels().all(|p| p.0[0] == 255));
    }

    #[test]
    fn detect_skin_leaves_non_skin_as_zero() {
        let img = DynamicImage::new_rgb8(4, 4);
        let mask = detect_skin(&img);
        assert!(mask.pixels().all(|p| p.0[0] == 0));
    }

    // ─── skin_pct_in_rect / skin_pct_outside_rect ─────────────

    #[test]
    fn skin_pct_in_rect_all_skin() {
        let mask = GrayImage::from_pixel(20, 20, Luma([255]));
        let pct = skin_pct_in_rect(&mask, 5, 5, 10, 10);
        assert!((pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn skin_pct_in_rect_no_skin() {
        let mask = GrayImage::from_pixel(20, 20, Luma([0]));
        let pct = skin_pct_in_rect(&mask, 5, 5, 10, 10);
        assert!(pct.abs() < 0.01);
    }

    #[test]
    fn skin_pct_outside_rect_zero_when_skin_only_inside() {
        let mut mask = GrayImage::new(20, 20);
        for y in 5..15 {
            for x in 5..15 {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
        let outside = skin_pct_outside_rect(&mask, 5, 5, 10, 10);
        assert!(outside.abs() < 0.01);
    }

    #[test]
    fn skin_pct_outside_rect_nonzero_when_skin_outside() {
        let mask = GrayImage::from_pixel(20, 20, Luma([255]));
        let outside = skin_pct_outside_rect(&mask, 5, 5, 10, 10);
        assert!(outside > 0.0);
    }

    #[test]
    fn skin_pct_in_and_outside_are_complementary() {
        let mut mask = GrayImage::new(20, 20);
        for y in 5..15 {
            for x in 5..15 {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
        let inside = skin_pct_in_rect(&mask, 5, 5, 10, 10);
        let outside = skin_pct_outside_rect(&mask, 5, 5, 10, 10);
        assert!((inside - 100.0).abs() < 0.01);
        assert!(outside.abs() < 0.01);
    }
}
