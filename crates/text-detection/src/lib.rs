#![warn(clippy::all, clippy::nursery)]

mod metrics;

pub use metrics::{DetectedText, TextDetectionResult};

use image::DynamicImage;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;

pub struct TextDetector {
    engine: OcrEngine,
}

#[derive(Debug)]
pub enum TextDetectorError {
    DetectionModelLoad(rten::LoadError),
    RecognitionModelLoad(rten::LoadError),
    EngineInit(String),
}

impl std::fmt::Display for TextDetectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DetectionModelLoad(err) => {
                write!(f, "failed to load embedded text detection model: {err}")
            }
            Self::RecognitionModelLoad(err) => {
                write!(f, "failed to load embedded text recognition model: {err}")
            }
            Self::EngineInit(msg) => write!(f, "failed to create OCR engine: {msg}"),
        }
    }
}

impl std::error::Error for TextDetectorError {}

impl TextDetector {
    pub fn from_bundled_models() -> Result<Self, TextDetectorError> {
        // The models are baked into the binary via `include_bytes!`, so there is no
        // external file to locate at runtime. `load_static_slice` is rten's intended
        // entry point for `include_bytes!`-embedded models.
        static DETECTION_MODEL: &[u8] = include_bytes!(env!("TEXT_DETECTION_MODEL_PATH"));
        static RECOGNITION_MODEL: &[u8] = include_bytes!(env!("TEXT_RECOGNITION_MODEL_PATH"));

        let detection_model = Model::load_static_slice(DETECTION_MODEL)
            .map_err(TextDetectorError::DetectionModelLoad)?;
        let recognition_model = Model::load_static_slice(RECOGNITION_MODEL)
            .map_err(TextDetectorError::RecognitionModelLoad)?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|e| TextDetectorError::EngineInit(e.to_string()))?;
        Ok(Self { engine })
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
    fn bundled_models_load() {
        // Guards the embedded-weights path: the models are baked into the binary via
        // `include_bytes!` and must be loadable by rten without any external file.
        TextDetector::from_bundled_models().expect("bundled text detection models should load");
    }
}
