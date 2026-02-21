use crate::faces::BoundingBox;

pub struct CandidateScore {
    pub face_position: f32,
    pub overlap: f32,
    pub avg_certainty: f32,
    pub overall: f32,
}

/// Returns 0.0 if the face center is in the center cell of a 3x3 grid, 1.0 otherwise.
fn face_position_score(face: &BoundingBox, image_width: u32, image_height: u32) -> f32 {
    let cx = (face.x1 + face.x2) / 2.0;
    let cy = (face.y1 + face.y2) / 2.0;

    let w = image_width as f32;
    let h = image_height as f32;

    let in_center_x = cx >= w / 3.0 && cx <= 2.0 * w / 3.0;
    let in_center_y = cy >= h / 3.0 && cy <= 2.0 * h / 3.0;

    if in_center_x && in_center_y {
        0.0
    } else {
        1.0
    }
}

/// Returns 1.0 - (face_area / image_area). Smaller faces relative to the image score higher.
fn overlap_score(face: &BoundingBox, image_width: u32, image_height: u32) -> f32 {
    let face_area = (face.x2 - face.x1) * (face.y2 - face.y1);
    let image_area = (image_width as f32) * (image_height as f32);
    if image_area == 0.0 {
        return 0.0;
    }
    (1.0 - face_area / image_area).clamp(0.0, 1.0)
}

/// Score a candidate image using the highest-confidence face.
pub fn score_candidate(
    faces: &[BoundingBox],
    image_width: u32,
    image_height: u32,
    landmark_confidence: f32,
) -> CandidateScore {
    let face = faces
        .iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
        .expect("score_candidate requires at least one face");

    let fp = face_position_score(face, image_width, image_height);
    let ol = overlap_score(face, image_width, image_height);
    let avg_cert = (face.confidence + landmark_confidence) / 2.0;
    let overall = avg_cert * fp * ol;

    CandidateScore {
        face_position: fp,
        overlap: ol,
        avg_certainty: avg_cert,
        overall,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(x1: f32, y1: f32, x2: f32, y2: f32, confidence: f32) -> BoundingBox {
        BoundingBox {
            x1,
            y1,
            x2,
            y2,
            confidence,
        }
    }

    #[test]
    fn face_in_center_gives_position_score_zero() {
        // Face centered in a 300x300 image → center cell
        let f = face(120.0, 120.0, 180.0, 180.0, 0.9);
        assert_eq!(face_position_score(&f, 300, 300), 0.0);
    }

    #[test]
    fn face_in_corner_gives_position_score_one() {
        // Face in top-left corner
        let f = face(10.0, 10.0, 50.0, 50.0, 0.9);
        assert_eq!(face_position_score(&f, 300, 300), 1.0);
    }

    #[test]
    fn small_face_gives_high_overlap_score() {
        // 10x10 face in 1000x1000 image → area ratio = 100/1_000_000 = 0.0001
        let f = face(0.0, 0.0, 10.0, 10.0, 0.9);
        let score = overlap_score(&f, 1000, 1000);
        assert!(score > 0.99);
    }

    #[test]
    fn large_face_gives_low_overlap_score() {
        // 900x900 face in 1000x1000 image → area ratio = 0.81
        let f = face(50.0, 50.0, 950.0, 950.0, 0.9);
        let score = overlap_score(&f, 1000, 1000);
        assert!(score < 0.2);
    }

    #[test]
    fn overall_score_is_product_of_components() {
        // Face in top-left corner (position=1.0), small face (overlap≈1.0)
        let f = face(10.0, 10.0, 20.0, 20.0, 0.8);
        let result = score_candidate(&[f], 1000, 1000, 0.6);

        let expected_avg = (0.8 + 0.6) / 2.0;
        let expected = expected_avg * result.face_position * result.overlap;
        assert!((result.overall - expected).abs() < 1e-6);
        assert_eq!(result.face_position, 1.0);
        assert_eq!(result.avg_certainty, expected_avg);
    }

    #[test]
    fn center_face_gives_zero_overall() {
        // Face in center → position=0.0 → overall=0.0 regardless of other scores
        let f = face(140.0, 140.0, 160.0, 160.0, 0.95);
        let result = score_candidate(&[f], 300, 300, 0.9);
        assert_eq!(result.overall, 0.0);
        assert_eq!(result.face_position, 0.0);
    }

    #[test]
    fn uses_highest_confidence_face() {
        let low = face(10.0, 10.0, 20.0, 20.0, 0.5); // corner
        let high = face(140.0, 140.0, 160.0, 160.0, 0.9); // center
        let result = score_candidate(&[low, high], 300, 300, 0.8);
        // Highest confidence face is in center → position=0.0
        assert_eq!(result.face_position, 0.0);
    }
}
