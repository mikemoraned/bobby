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
    let img_w = mask.width();
    let img_h = mask.height();

    let x_end = (x + w).min(img_w);
    let y_end = (y + h).min(img_h);
    let x_start = x.min(img_w);
    let y_start = y.min(img_h);

    let mut total = 0u32;
    let mut skin = 0u32;

    for py in y_start..y_end {
        for px in x_start..x_end {
            total += 1;
            if mask.get_pixel(px, py).0[0] > 0 {
                skin += 1;
            }
        }
    }

    if total == 0 {
        return 0.0;
    }

    (skin as f32 / total as f32) * 100.0
}

/// Compute the percentage of skin pixels outside a rectangular region.
pub fn skin_pct_outside_rect(mask: &GrayImage, x: u32, y: u32, w: u32, h: u32) -> f32 {
    let img_w = mask.width();
    let img_h = mask.height();

    let x_end = (x + w).min(img_w);
    let y_end = (y + h).min(img_h);
    let x_start = x.min(img_w);
    let y_start = y.min(img_h);

    let mut total = 0u32;
    let mut skin = 0u32;

    for py in 0..img_h {
        for px in 0..img_w {
            let inside = px >= x_start && px < x_end && py >= y_start && py < y_end;
            if !inside {
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
}
