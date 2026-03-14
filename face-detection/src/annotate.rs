use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_hollow_rect_mut, draw_line_segment_mut};
use imageproc::rect::Rect;

use crate::Face;

const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);

pub fn annotate_image(image: &DynamicImage, face: &Face) -> DynamicImage {
    let mut canvas: RgbaImage = image.to_rgba8();
    let (img_w, img_h) = (canvas.width() as i32, canvas.height() as i32);

    let x = face.x as i32;
    let y = face.y as i32;
    let w = face.width as i32;
    let h = face.height as i32;

    // Bounding box
    if w > 0 && h > 0 {
        draw_hollow_rect_mut(
            &mut canvas,
            Rect::at(x, y).of_size(w as u32, h as u32),
            RED,
        );
    }

    // Crosshairs centred on bounding box centre
    let cx = face.x + face.width / 2.0;
    let cy = face.y + face.height / 2.0;
    let box_left = face.x;
    let box_right = face.x + face.width;
    let box_top = face.y;
    let box_bottom = face.y + face.height;

    // Horizontal: left edge → box left, box right → right edge
    draw_line_segment_mut(&mut canvas, (0.0, cy), (box_left, cy), RED);
    draw_line_segment_mut(&mut canvas, (box_right, cy), (img_w as f32, cy), RED);

    // Vertical: top edge → box top, box bottom → bottom edge
    draw_line_segment_mut(&mut canvas, (cx, 0.0), (cx, box_top), RED);
    draw_line_segment_mut(&mut canvas, (cx, box_bottom), (cx, img_h as f32), RED);

    DynamicImage::ImageRgba8(canvas)
}
