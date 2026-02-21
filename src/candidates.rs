use std::fmt;
use std::path::{Path, PathBuf};

use image::{DynamicImage, Rgb, RgbImage};

use crate::faces::BoundingBox;

const CANDIDATES_DIR: &str = "candidates";
const FACE_COLOR: Rgb<u8> = Rgb([255, 0, 0]);
const LANDMARK_COLOR: Rgb<u8> = Rgb([0, 255, 0]);
const BOX_THICKNESS: u32 = 3;
const BORDER_THICKNESS: u32 = 4;

#[derive(Debug, Clone)]
pub struct CandidateId(String);

impl CandidateId {
    pub fn new(did: &str, rkey: &str, image_index: usize) -> Self {
        // Strip the "did:plc:" prefix for shorter filenames, keep just the identifier.
        let short_did = did.strip_prefix("did:plc:").unwrap_or(did);
        Self(format!("{short_did}_{rkey}_{image_index}"))
    }
}

impl fmt::Display for CandidateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("failed to create candidates directory: {0}")]
    CreateDir(std::io::Error),

    #[error("failed to save image: {0}")]
    SaveImage(image::ImageError),
}

pub struct SavedCandidate {
    pub id: CandidateId,
    pub original_path: PathBuf,
    pub annotated_path: PathBuf,
}

pub fn save_candidate(
    img: &DynamicImage,
    faces: &[BoundingBox],
    landmark_detected: bool,
    id: &CandidateId,
) -> Result<SavedCandidate, SaveError> {
    let dir = Path::new(CANDIDATES_DIR);
    std::fs::create_dir_all(dir).map_err(SaveError::CreateDir)?;

    let original_path = dir.join(format!("{id}.png"));
    img.save(&original_path).map_err(SaveError::SaveImage)?;

    let annotated = draw_annotations(img, faces, landmark_detected);
    let annotated_path = dir.join(format!("{id}_annotated.png"));
    annotated.save(&annotated_path).map_err(SaveError::SaveImage)?;

    Ok(SavedCandidate {
        id: id.clone(),
        original_path,
        annotated_path,
    })
}

fn draw_rect(canvas: &mut RgbImage, x1: u32, y1: u32, x2: u32, y2: u32, thickness: u32, color: Rgb<u8>) {
    // Draw top and bottom horizontal lines
    for t in 0..thickness {
        let yt = y1.saturating_add(t).min(canvas.height().saturating_sub(1));
        let yb = y2.saturating_sub(t).max(y1);
        for x in x1..=x2 {
            canvas.put_pixel(x, yt, color);
            canvas.put_pixel(x, yb, color);
        }
    }

    // Draw left and right vertical lines
    for t in 0..thickness {
        let xl = x1.saturating_add(t).min(canvas.width().saturating_sub(1));
        let xr = x2.saturating_sub(t).max(x1);
        for y in y1..=y2 {
            canvas.put_pixel(xl, y, color);
            canvas.put_pixel(xr, y, color);
        }
    }
}

fn draw_annotations(img: &DynamicImage, faces: &[BoundingBox], landmark_detected: bool) -> DynamicImage {
    let mut canvas: RgbImage = img.to_rgb8();
    let w = canvas.width();
    let h = canvas.height();

    // Draw green border around entire image if landmark detected
    if landmark_detected && w > 0 && h > 0 {
        draw_rect(&mut canvas, 0, 0, w - 1, h - 1, BORDER_THICKNESS, LANDMARK_COLOR);
    }

    // Draw red bounding boxes for faces
    for face in faces {
        let x1 = (face.x1 as u32).min(w.saturating_sub(1));
        let y1 = (face.y1 as u32).min(h.saturating_sub(1));
        let x2 = (face.x2 as u32).min(w.saturating_sub(1));
        let y2 = (face.y2 as u32).min(h.saturating_sub(1));
        draw_rect(&mut canvas, x1, y1, x2, y2, BOX_THICKNESS, FACE_COLOR);
    }

    DynamicImage::ImageRgb8(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_id_strips_did_prefix() {
        let id = CandidateId::new("did:plc:abc123", "xyz789", 0);
        assert_eq!(id.to_string(), "abc123_xyz789_0");
    }

    #[test]
    fn candidate_id_handles_non_plc_did() {
        let id = CandidateId::new("did:web:example.com", "rk1", 2);
        assert_eq!(id.to_string(), "did:web:example.com_rk1_2");
    }

    #[test]
    fn draw_annotations_produces_red_face_boxes() {
        let img = DynamicImage::new_rgb8(100, 100);
        let faces = vec![BoundingBox {
            x1: 10.0,
            y1: 10.0,
            x2: 50.0,
            y2: 50.0,
            confidence: 0.9,
        }];
        let annotated = draw_annotations(&img, &faces, false);
        let rgb = annotated.to_rgb8();

        // Top edge should have red pixels
        assert_eq!(*rgb.get_pixel(30, 10), FACE_COLOR);
        // Left edge should have red pixels
        assert_eq!(*rgb.get_pixel(10, 30), FACE_COLOR);
        // Interior should remain black (original was blank)
        assert_eq!(*rgb.get_pixel(30, 30), Rgb([0, 0, 0]));
    }

    #[test]
    fn draw_annotations_with_landmark_has_green_border() {
        let img = DynamicImage::new_rgb8(100, 100);
        let faces = vec![BoundingBox {
            x1: 20.0,
            y1: 20.0,
            x2: 40.0,
            y2: 40.0,
            confidence: 0.9,
        }];
        let annotated = draw_annotations(&img, &faces, true);
        let rgb = annotated.to_rgb8();

        // Top-left corner should be green (landmark border)
        assert_eq!(*rgb.get_pixel(0, 0), LANDMARK_COLOR);
        // Bottom-right corner should be green
        assert_eq!(*rgb.get_pixel(99, 99), LANDMARK_COLOR);
        // Mid-edge should be green
        assert_eq!(*rgb.get_pixel(50, 0), LANDMARK_COLOR);
        // Face box should still be red (drawn on top)
        assert_eq!(*rgb.get_pixel(30, 20), FACE_COLOR);
    }

    #[test]
    fn draw_and_encode_produces_valid_png() {
        let img = DynamicImage::new_rgb8(64, 64);
        let faces = vec![BoundingBox {
            x1: 5.0,
            y1: 5.0,
            x2: 20.0,
            y2: 20.0,
            confidence: 0.8,
        }];
        let annotated = draw_annotations(&img, &faces, true);
        let mut out = std::io::Cursor::new(Vec::new());
        annotated
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        assert!(!out.get_ref().is_empty());
    }
}
