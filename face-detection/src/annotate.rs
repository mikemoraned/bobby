use image::{DynamicImage, GrayImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_hollow_rect_mut, draw_line_segment_mut};
use imageproc::rect::Rect;

use crate::Face;

const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
const SKIN_OVERLAY: Rgba<u8> = Rgba([0, 200, 100, 128]);

pub fn annotate_image(
    image: &DynamicImage,
    face: &Face,
    skin_mask: &GrayImage,
) -> DynamicImage {
    let mut canvas: RgbaImage = image.to_rgba8();
    let (img_w, img_h) = (canvas.width() as i32, canvas.height() as i32);

    // Skin mask overlay at 50% opacity
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        if skin_mask.get_pixel(x, y).0[0] > 0 {
            pixel[0] = ((u16::from(pixel[0]) + u16::from(SKIN_OVERLAY[0])) / 2) as u8;
            pixel[1] = ((u16::from(pixel[1]) + u16::from(SKIN_OVERLAY[1])) / 2) as u8;
            pixel[2] = ((u16::from(pixel[2]) + u16::from(SKIN_OVERLAY[2])) / 2) as u8;
        }
    }

    let x = face.x as i32;
    let y = face.y as i32;
    let w = face.width as i32;
    let h = face.height as i32;

    // Face bounding box
    if w > 0 && h > 0 {
        draw_hollow_rect_mut(
            &mut canvas,
            Rect::at(x, y).of_size(w as u32, h as u32),
            RED,
        );
    }

    // Crosshairs centred on face bounding box centre
    let cx = face.x + face.width / 2.0;
    let cy = face.y + face.height / 2.0;
    let box_left = face.x;
    let box_right = face.x + face.width;
    let box_top = face.y;
    let box_bottom = face.y + face.height;

    draw_line_segment_mut(&mut canvas, (0.0, cy), (box_left, cy), RED);
    draw_line_segment_mut(&mut canvas, (box_right, cy), (img_w as f32, cy), RED);
    draw_line_segment_mut(&mut canvas, (cx, 0.0), (cx, box_top), RED);
    draw_line_segment_mut(&mut canvas, (cx, box_bottom), (cx, img_h as f32), RED);

    DynamicImage::ImageRgba8(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Landmarks;
    use image::GrayImage;
    use image::Luma;

    fn test_face() -> Face {
        Face {
            x: 20.0,
            y: 20.0,
            width: 60.0,
            height: 60.0,
            score: 0.9,
            landmarks: Landmarks {
                right_eye: (35.0, 35.0),
                left_eye: (65.0, 35.0),
                nose: (50.0, 50.0),
                right_mouth: (40.0, 60.0),
                left_mouth: (60.0, 60.0),
            },
        }
    }

    #[test]
    fn annotate_preserves_dimensions() {
        let img = DynamicImage::new_rgb8(100, 100);
        let mask = GrayImage::new(100, 100);
        let result = annotate_image(&img, &test_face(), &mask);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn annotate_produces_rgba_output() {
        let img = DynamicImage::new_rgb8(100, 100);
        let mask = GrayImage::new(100, 100);
        let result = annotate_image(&img, &test_face(), &mask);
        assert!(result.as_rgba8().is_some());
    }

    #[test]
    fn annotate_skin_overlay_changes_pixels() {
        // White image with all-skin mask → overlay should tint pixels
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            100, 100, image::Rgb([255, 255, 255]),
        ));
        let mask = GrayImage::from_pixel(100, 100, Luma([255]));
        let result = annotate_image(&img, &test_face(), &mask);
        let rgba = result.as_rgba8().expect("rgba");
        // The overlay blends with SKIN_OVERLAY (0, 200, 100, 128)
        // For white (255, 255, 255): r = (255+0)/2=127, g = (255+200)/2=227, b = (255+100)/2=177
        let pixel = rgba.get_pixel(0, 0);
        assert!(pixel[0] < 200, "red channel should be tinted down");
        assert!(pixel[1] > 200, "green channel should stay high");
    }

    #[test]
    fn annotate_no_skin_leaves_pixels_untinted() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            100, 100, image::Rgb([255, 255, 255]),
        ));
        let mask = GrayImage::from_pixel(100, 100, Luma([0]));
        let result = annotate_image(&img, &test_face(), &mask);
        let rgba = result.as_rgba8().expect("rgba");
        // Non-face pixels with no skin should remain white-ish
        // (face bbox pixels will have red lines drawn on them)
        // Check a corner pixel that's outside the crosshairs
        let pixel = rgba.get_pixel(99, 0);
        // This pixel is at the right edge, top — the horizontal crosshair
        // passes at cy≈50 and vertical at cx≈50, so (99,0) is outside both
        // BUT the horizontal line at cy goes all the way to img_w at cy,
        // and vertical line goes all the way. Actually (99,0) is not on
        // any crosshair since cy=50 and cx=50.
        assert_eq!(pixel[0], 255);
        assert_eq!(pixel[1], 255);
        assert_eq!(pixel[2], 255);
    }

    #[test]
    fn annotate_draws_bounding_box() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            100, 100, image::Rgb([255, 255, 255]),
        ));
        let mask = GrayImage::new(100, 100);
        let result = annotate_image(&img, &test_face(), &mask);
        let rgba = result.as_rgba8().expect("rgba");
        // The bounding box edge at (20, 20) should have red drawn on it
        let pixel = rgba.get_pixel(20, 20);
        assert_eq!(pixel[0], 255, "red channel should be max on bbox");
        assert_eq!(pixel[1], 0, "green should be 0 on red bbox");
    }
}
