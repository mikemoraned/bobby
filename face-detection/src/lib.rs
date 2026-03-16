#![warn(clippy::all, clippy::nursery)]

mod annotate;
mod postprocess;
mod preprocess;

pub use annotate::{TextRegion, annotate_image};

pub mod model {
    #![allow(clippy::type_complexity)]
    include!(concat!(
        env!("OUT_DIR"),
        "/model/face_detection_yunet_2023mar_opset16.rs"
    ));
}

use burn::backend::NdArray;
use euclid::default::{Point2D, Rect, Size2D, Vector2D};
use image::DynamicImage;
use postprocess::{Detection, decode_and_filter};
use preprocess::image_to_tensor;
pub use shared::{Percentage, Zone};

type Backend = NdArray;

const SCORE_THRESHOLD: f32 = 0.6;
const NMS_IOU_THRESHOLD: f32 = 0.3;

pub struct FaceDetector {
    model: model::Model<Backend>,
}

#[derive(Debug, Clone)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub score: f32,
    pub landmarks: Landmarks,
}

#[derive(Debug, Clone)]
pub struct Landmarks {
    pub right_eye: (f32, f32),
    pub left_eye: (f32, f32),
    pub nose: (f32, f32),
    pub right_mouth: (f32, f32),
    pub left_mouth: (f32, f32),
}

impl Face {
    pub fn is_frontal(&self) -> bool {
        let (rx, ry) = self.landmarks.right_eye;
        let (lx, ly) = self.landmarks.left_eye;

        let eye_dx = lx - rx;
        let eye_dy = ly - ry;
        let eye_distance = eye_dx.hypot(eye_dy);

        if eye_distance < 1.0 {
            return false;
        }

        // Eyes should be roughly horizontal (angle < 30 degrees)
        let angle = (eye_dy / eye_dx).atan().abs();
        if angle > std::f32::consts::FRAC_PI_6 {
            return false;
        }

        // Nose should be roughly centered between eyes horizontally
        let eye_center_x = (rx + lx) / 2.0;
        let nose_x = self.landmarks.nose.0;
        let nose_offset = ((nose_x - eye_center_x) / eye_distance).abs();
        nose_offset < 0.35
    }

    /// Face area as a percentage of the total image area.
    ///
    /// The face bounding box is clipped to the image boundaries so that
    /// regions extending outside the image are not counted.
    pub fn area_pct(&self, image_width: u32, image_height: u32) -> Percentage {
        let image_area = image_width as f32 * image_height as f32;
        let face_rect = Rect::new(
            Point2D::new(self.x, self.y),
            Size2D::new(self.width, self.height),
        );
        let image_rect = Rect::new(
            Point2D::origin(),
            Size2D::new(image_width as f32, image_height as f32),
        );
        let clipped_area = face_rect
            .intersection(&image_rect)
            .map_or(0.0, |r| r.area());
        Percentage::new((clipped_area / image_area) * 100.0)
    }
}

impl FaceDetector {
    pub fn new(weights_path: &str) -> Self {
        let device = Default::default();
        let model = model::Model::<Backend>::from_file(weights_path, &device);
        Self { model }
    }

    pub fn from_bundled_weights() -> Self {
        Self::new(env!("YUNET_WEIGHTS_PATH"))
    }

    pub fn detect(&self, image: &DynamicImage) -> Vec<Face> {
        let (tensor, scale_x, scale_y) = image_to_tensor(image);
        let outputs = self.model.forward(tensor);

        let detections =
            decode_and_filter(outputs, SCORE_THRESHOLD, NMS_IOU_THRESHOLD);

        detections
            .into_iter()
            .map(|d| to_face(d, scale_x, scale_y))
            .collect()
    }
}

fn to_face(d: Detection, scale_x: f32, scale_y: f32) -> Face {
    Face {
        x: d.x * scale_x,
        y: d.y * scale_y,
        width: d.width * scale_x,
        height: d.height * scale_y,
        score: d.score,
        landmarks: Landmarks {
            right_eye: (d.landmarks[0] * scale_x, d.landmarks[1] * scale_y),
            left_eye: (d.landmarks[2] * scale_x, d.landmarks[3] * scale_y),
            nose: (d.landmarks[4] * scale_x, d.landmarks[5] * scale_y),
            right_mouth: (d.landmarks[6] * scale_x, d.landmarks[7] * scale_y),
            left_mouth: (d.landmarks[8] * scale_x, d.landmarks[9] * scale_y),
        },
    }
}

fn overlap_area(a: &Rect<f32>, b: &Rect<f32>) -> f32 {
    a.intersection(b).map_or(0.0, |r| r.area())
}

/// Map a face to a zone by measuring overlap with each of the 9 zones
/// and choosing the zone with maximum overlap.
///
/// Zones are defined on a 4x4 grid where each zone is a 2x2 block.
/// One grid unit on X = width/4, one grid unit on Y = height/4.
/// Valid offsets are 0, 1, 2 in each axis, giving 9 zones.
pub fn face_zone(face: &Face, image_width: u32, image_height: u32) -> Zone {
    let unit_w = image_width as f32 / 4.0;
    let unit_h = image_height as f32 / 4.0;
    let face_rect = Rect::new(
        Point2D::new(face.x, face.y),
        Size2D::new(face.width, face.height),
    );

    let base = Rect::new(Point2D::origin(), Size2D::new(unit_w * 2.0, unit_h * 2.0));
    let zones: [(Zone, Rect<f32>); 9] = [
        (Zone::TopLeft,      base.translate(Vector2D::new(0.0,          0.0))),
        (Zone::TopCenter,    base.translate(Vector2D::new(unit_w,       0.0))),
        (Zone::TopRight,     base.translate(Vector2D::new(unit_w * 2.0, 0.0))),
        (Zone::CenterLeft,   base.translate(Vector2D::new(0.0,          unit_h))),
        (Zone::CenterCenter, base.translate(Vector2D::new(unit_w,       unit_h))),
        (Zone::CenterRight,  base.translate(Vector2D::new(unit_w * 2.0, unit_h))),
        (Zone::BottomLeft,   base.translate(Vector2D::new(0.0,          unit_h * 2.0))),
        (Zone::BottomCenter, base.translate(Vector2D::new(unit_w,       unit_h * 2.0))),
        (Zone::BottomRight,  base.translate(Vector2D::new(unit_w * 2.0, unit_h * 2.0))),
    ];

    zones
        .into_iter()
        .max_by(|(_, a), (_, b)| {
            overlap_area(&face_rect, a)
                .partial_cmp(&overlap_area(&face_rect, b))
                .expect("non-NaN overlap values")
        })
        .expect("zones is non-empty")
        .0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_face(x: f32, y: f32, w: f32, h: f32) -> Face {
        Face {
            x,
            y,
            width: w,
            height: h,
            score: 0.9,
            landmarks: Landmarks {
                right_eye: (x + w * 0.3, y + h * 0.35),
                left_eye: (x + w * 0.7, y + h * 0.35),
                nose: (x + w * 0.5, y + h * 0.55),
                right_mouth: (x + w * 0.35, y + h * 0.7),
                left_mouth: (x + w * 0.65, y + h * 0.7),
            },
        }
    }

    #[test]
    fn frontal_face_detected_as_frontal() {
        let face = make_face(100.0, 100.0, 200.0, 200.0);
        assert!(face.is_frontal());
    }

    #[test]
    fn side_profile_not_frontal() {
        let mut face = make_face(100.0, 100.0, 200.0, 200.0);
        face.landmarks.nose = (face.x + face.width * 0.9, face.y + face.height * 0.55);
        assert!(!face.is_frontal());
    }

    #[test]
    fn zone_top_right() {
        // Face fully in the right half of the top row (x offset 2, y offset 0)
        let face = make_face(400.0, 20.0, 100.0, 100.0);
        assert_eq!(face_zone(&face, 640, 480), Zone::TopRight);
    }

    #[test]
    fn zone_bottom_left() {
        // Face in the left column, bottom row
        let face = make_face(20.0, 300.0, 100.0, 100.0);
        assert_eq!(face_zone(&face, 640, 480), Zone::BottomLeft);
    }

    #[test]
    fn zone_center_center() {
        // Face in the middle of the image
        let face = make_face(220.0, 140.0, 200.0, 200.0);
        assert_eq!(face_zone(&face, 640, 480), Zone::CenterCenter);
    }

    #[test]
    fn zone_top_center() {
        // Face centered horizontally at the top — the key new rejection zone
        // For 640 wide: unit_w = 160, so TopCenter covers x=[160, 480], y=[0, 240]
        let face = make_face(220.0, 20.0, 100.0, 100.0);
        assert_eq!(face_zone(&face, 640, 480), Zone::TopCenter);
    }

    #[test]
    fn area_pct_capped_at_100_when_bbox_exceeds_image() {
        // Face bbox extends well beyond the 100x100 image
        let face = make_face(-10.0, -10.0, 200.0, 200.0);
        let pct = face.area_pct(100, 100);
        assert!(
            pct.value() <= 100.0,
            "area_pct should be capped at 100.0, got {}",
            pct.value()
        );
    }
}
