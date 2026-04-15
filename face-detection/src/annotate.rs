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
