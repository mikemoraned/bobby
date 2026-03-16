#![warn(clippy::all, clippy::nursery)]

use geo::{Area, BooleanOps, Rect as GeoRect, coord};
use image::DynamicImage;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;

pub struct TextDetector {
    engine: OcrEngine,
}

/// A region of detected text with its bounding box and recognized content.
#[derive(Debug, Clone)]
pub struct DetectedText {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub text: String,
}

/// Result of running text detection on an image.
#[derive(Debug, Clone)]
pub struct TextDetectionResult {
    pub lines: Vec<DetectedText>,
}

impl TextDetectionResult {
    /// Count the total number of non-whitespace characters across all detected lines.
    pub fn character_count(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum()
    }

    /// Calculate the percentage of the image area covered by detected text bounding boxes.
    ///
    /// Overlapping text regions are unioned so that shared pixels are only
    /// counted once, preventing the result from exceeding 100%.
    pub fn text_area_pct(&self, image_width: u32, image_height: u32) -> f32 {
        let image_area = f64::from(image_width) * f64::from(image_height);
        if image_area == 0.0 {
            return 0.0;
        }

        let image_rect = GeoRect::new(
            coord! { x: 0.0, y: 0.0 },
            coord! { x: f64::from(image_width), y: f64::from(image_height) },
        );

        let polygons: Vec<_> = self
            .lines
            .iter()
            .filter(|line| line.width > 0 && line.height > 0)
            .filter_map(|line| {
                let text_rect = GeoRect::new(
                    coord! { x: f64::from(line.x), y: f64::from(line.y) },
                    coord! {
                        x: f64::from(line.x + line.width),
                        y: f64::from(line.y + line.height)
                    },
                );
                let clipped = text_rect.to_polygon().intersection(&image_rect.to_polygon());
                if clipped.unsigned_area() > 0.0 {
                    Some(clipped)
                } else {
                    None
                }
            })
            .collect();

        if polygons.is_empty() {
            return 0.0;
        }

        let mut union = polygons[0].clone();
        for poly in &polygons[1..] {
            union = union.union(poly);
        }

        (union.unsigned_area() / image_area * 100.0) as f32
    }

    /// Join all detected text into a single string, separated by newlines.
    pub fn full_text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl TextDetector {
    pub fn new(detection_model_path: &str, recognition_model_path: &str) -> Self {
        let detection_model =
            Model::load_file(detection_model_path).expect("failed to load text detection model");
        let recognition_model = Model::load_file(recognition_model_path)
            .expect("failed to load text recognition model");
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .expect("failed to create OCR engine");
        Self { engine }
    }

    pub fn from_bundled_models() -> Self {
        Self::new(
            env!("TEXT_DETECTION_MODEL_PATH"),
            env!("TEXT_RECOGNITION_MODEL_PATH"),
        )
    }

    /// Detect and recognize text in the image, returning bounding boxes and text.
    pub fn detect(&self, image: &DynamicImage) -> TextDetectionResult {
        let rgb = image.to_rgb8();
        let Ok(img_source) = ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions()) else {
            return TextDetectionResult { lines: Vec::new() };
        };
        let Ok(ocr_input) = self.engine.prepare_input(img_source) else {
            return TextDetectionResult { lines: Vec::new() };
        };

        let Ok(words) = self.engine.detect_words(&ocr_input) else {
            return TextDetectionResult { lines: Vec::new() };
        };

        let word_lines = self.engine.find_text_lines(&ocr_input, &words);

        let Ok(text_lines) = self.engine.recognize_text(&ocr_input, &word_lines) else {
            return TextDetectionResult { lines: Vec::new() };
        };

        let lines = text_lines
            .into_iter()
            .flatten()
            .map(|line| {
                let text: String = line.to_string();
                let rect = line.bounding_rect();
                DetectedText {
                    x: rect.left(),
                    y: rect.top(),
                    width: rect.width(),
                    height: rect.height(),
                    text,
                }
            })
            .collect();

        TextDetectionResult { lines }
    }

    /// Count the number of recognized text characters in the image.
    pub fn count_characters(&self, image: &DynamicImage) -> usize {
        self.detect(image).character_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_area_pct_capped_at_100_with_overlapping_boxes() {
        let result = TextDetectionResult {
            lines: vec![
                DetectedText { x: 0, y: 0, width: 100, height: 100, text: "A".into() },
                DetectedText { x: 10, y: 10, width: 100, height: 100, text: "B".into() },
            ],
        };
        let pct = result.text_area_pct(100, 100);
        assert!(
            pct <= 100.0,
            "text_area_pct should be capped at 100.0, got {pct}"
        );
    }

    #[test]
    fn text_area_pct_zero_image_returns_zero() {
        let result = TextDetectionResult {
            lines: vec![DetectedText { x: 0, y: 0, width: 10, height: 10, text: "A".into() }],
        };
        assert_eq!(result.text_area_pct(0, 0), 0.0);
    }
}
