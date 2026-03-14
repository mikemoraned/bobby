#![warn(clippy::all, clippy::nursery)]

mod annotate;
mod postprocess;
mod preprocess;

pub use annotate::annotate_image;

pub mod model {
    #![allow(clippy::type_complexity)]
    include!(concat!(
        env!("OUT_DIR"),
        "/model/face_detection_yunet_2023mar_opset16.rs"
    ));
}

use burn::backend::NdArray;
use image::DynamicImage;
use postprocess::{Detection, decode_and_filter};
use preprocess::image_to_tensor;

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

    fn center(&self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quadrant {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl std::fmt::Display for Quadrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopLeft => write!(f, "TOP_LEFT"),
            Self::TopRight => write!(f, "TOP_RIGHT"),
            Self::BottomLeft => write!(f, "BOTTOM_LEFT"),
            Self::BottomRight => write!(f, "BOTTOM_RIGHT"),
        }
    }
}

pub fn face_quadrant(face: &Face, image_width: u32, image_height: u32) -> Quadrant {
    let (cx, cy) = face.center();
    let mid_x = image_width as f32 / 2.0;
    let mid_y = image_height as f32 / 2.0;

    match (cx < mid_x, cy < mid_y) {
        (true, true) => Quadrant::TopLeft,
        (false, true) => Quadrant::TopRight,
        (true, false) => Quadrant::BottomLeft,
        (false, false) => Quadrant::BottomRight,
    }
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
    fn quadrant_top_right() {
        let face = make_face(350.0, 50.0, 100.0, 100.0);
        assert_eq!(face_quadrant(&face, 640, 480), Quadrant::TopRight);
    }

    #[test]
    fn quadrant_bottom_left() {
        let face = make_face(50.0, 300.0, 100.0, 100.0);
        assert_eq!(face_quadrant(&face, 640, 480), Quadrant::BottomLeft);
    }

}
