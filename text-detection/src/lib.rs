#![warn(clippy::all, clippy::nursery)]

use image::DynamicImage;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;

pub struct TextDetector {
    engine: OcrEngine,
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

    /// Count the number of recognized text characters in the image.
    pub fn count_characters(&self, image: &DynamicImage) -> usize {
        let rgb = image.to_rgb8();
        let Ok(img_source) = ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions()) else {
            return 0;
        };
        let Ok(ocr_input) = self.engine.prepare_input(img_source) else {
            return 0;
        };
        let Ok(text) = self.engine.get_text(&ocr_input) else {
            return 0;
        };
        text.chars().filter(|c| !c.is_whitespace()).count()
    }
}
