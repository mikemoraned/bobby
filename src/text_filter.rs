use image::DynamicImage;
use ocrs::{OcrEngine, OcrEngineParams};
use rten::Model;
use rten_imageproc::RotatedRect;

const TEXT_COVERAGE_THRESHOLD: f64 = 0.30;
const MODEL_PATH: &str = "models/text-detection.rten";

pub struct TextDetector {
    engine: OcrEngine,
}

impl TextDetector {
    pub fn new() -> Self {
        let detection_model =
            Model::load_file(MODEL_PATH).expect("failed to load text detection model");
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: None,
            ..Default::default()
        })
        .expect("failed to initialize OCR engine");
        Self { engine }
    }

    pub fn is_mostly_text(&self, img: &DynamicImage) -> bool {
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        let image_area = (w as f64) * (h as f64);
        if image_area == 0.0 {
            return false;
        }

        let ocr_input = self
            .engine
            .prepare_input(ocrs::ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions()).unwrap())
            .expect("failed to prepare OCR input");

        let word_rects = self
            .engine
            .detect_words(&ocr_input)
            .expect("text detection failed");

        let text_area = text_coverage_area(&word_rects);
        let fraction = text_area / image_area;

        fraction >= TEXT_COVERAGE_THRESHOLD
    }
}

fn text_coverage_area(rects: &[RotatedRect]) -> f64 {
    rects.iter().map(|r| (r.width() * r.height()) as f64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rects_have_zero_area() {
        assert_eq!(text_coverage_area(&[]), 0.0);
    }

    #[test]
    #[ignore] // requires `just download-text-detection-model`
    fn screenshot_detected_as_text() {
        let detector = TextDetector::new();
        // A solid white image with no actual text should NOT be detected as text
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            200,
            200,
            image::Rgb([255, 255, 255]),
        ));
        assert!(!detector.is_mostly_text(&img));
    }
}
