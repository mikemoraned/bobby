#![warn(clippy::all, clippy::nursery)]

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
    pub fn text_area_pct(&self, image_width: u32, image_height: u32) -> f32 {
        let image_area = f64::from(image_width) * f64::from(image_height);
        if image_area == 0.0 {
            return 0.0;
        }
        let text_area: f64 = self
            .lines
            .iter()
            .map(|line| f64::from(line.width.max(0)) * f64::from(line.height.max(0)))
            .sum();
        (text_area / image_area * 100.0) as f32
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
