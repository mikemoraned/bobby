use burn::backend::NdArray;
use burn::prelude::*;

#[allow(clippy::type_complexity)]
mod places365 {
    include!(concat!(env!("OUT_DIR"), "/model/places365.rs"));
}

type B = NdArray<f32>;

const INPUT_SIZE: usize = 224;
const NUM_CLASSES: usize = 365;
const CONFIDENCE_THRESHOLD: f32 = 0.1;

// ImageNet normalization constants
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Places365 category indices that correspond to landmark scenes.
const LANDMARK_INDICES: [usize; 28] = [
    12,  // arch
    13,  // archaeological_excavation
    16,  // arena/performance
    84,  // castle
    85,  // catacomb
    90,  // church/indoor
    91,  // church/outdoor
    154, // fountain
    163, // glacier
    214, // lighthouse
    226, // mausoleum
    230, // mosque/outdoor
    232, // mountain
    236, // museum/indoor
    237, // museum/outdoor
    239, // natural_history_museum
    242, // oast_house
    251, // pagoda
    252, // palace
    266, // pier
    270, // plaza
    289, // rock_arch
    292, // ruin
    297, // science_museum
    330, // temple/asia
    334, // tower
    350, // volcano
    355, // waterfall
];

/// Category names corresponding to each of the 365 Places365 classes.
/// Only landmark-related names are explicitly named; rest are empty placeholders
/// that we don't need to display.
const LANDMARK_NAMES: [(usize, &str); 28] = [
    (12, "arch"),
    (13, "archaeological excavation"),
    (16, "arena"),
    (84, "castle"),
    (85, "catacomb"),
    (90, "church (indoor)"),
    (91, "church (outdoor)"),
    (154, "fountain"),
    (163, "glacier"),
    (214, "lighthouse"),
    (226, "mausoleum"),
    (230, "mosque"),
    (232, "mountain"),
    (236, "museum (indoor)"),
    (237, "museum (outdoor)"),
    (239, "natural history museum"),
    (242, "oast house"),
    (251, "pagoda"),
    (252, "palace"),
    (266, "pier"),
    (270, "plaza"),
    (289, "rock arch"),
    (292, "ruin"),
    (297, "science museum"),
    (330, "temple"),
    (334, "tower"),
    (350, "volcano"),
    (355, "waterfall"),
];

#[derive(Debug, Clone)]
pub struct SceneClassification {
    pub category: String,
    pub confidence: f32,
    pub is_landmark: bool,
}

pub struct LandmarkDetector {
    model: places365::Model<B>,
    device: <B as Backend>::Device,
}

impl LandmarkDetector {
    pub fn new() -> Self {
        let device = Default::default();
        let model: places365::Model<B> = Default::default();
        Self { model, device }
    }

    pub fn classify(&self, img: &image::DynamicImage) -> SceneClassification {
        let input = preprocess(img, &self.device);
        let output = self.model.forward(input);
        postprocess(output)
    }
}

fn preprocess(img: &image::DynamicImage, device: &<B as Backend>::Device) -> Tensor<B, 4> {
    let resized = img.resize_exact(
        INPUT_SIZE as u32,
        INPUT_SIZE as u32,
        image::imageops::FilterType::Triangle,
    );
    let rgb = resized.to_rgb8();

    let channel_size = INPUT_SIZE * INPUT_SIZE;
    let mut data = vec![0.0f32; 3 * channel_size];

    for y in 0..INPUT_SIZE {
        for x in 0..INPUT_SIZE {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            let offset = y * INPUT_SIZE + x;
            // RGB channel order, ImageNet normalization
            data[offset] = (pixel[0] as f32 / 255.0 - MEAN[0]) / STD[0];
            data[channel_size + offset] = (pixel[1] as f32 / 255.0 - MEAN[1]) / STD[1];
            data[2 * channel_size + offset] = (pixel[2] as f32 / 255.0 - MEAN[2]) / STD[2];
        }
    }

    Tensor::<B, 1>::from_floats(data.as_slice(), device).reshape([1, 3, INPUT_SIZE, INPUT_SIZE])
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    exps.into_iter().map(|e| e / sum).collect()
}

fn is_landmark_index(idx: usize) -> bool {
    LANDMARK_INDICES.contains(&idx)
}

fn landmark_name(idx: usize) -> &'static str {
    LANDMARK_NAMES
        .iter()
        .find(|(i, _)| *i == idx)
        .map(|(_, name)| *name)
        .unwrap_or("unknown")
}

fn postprocess(output: Tensor<B, 2>) -> SceneClassification {
    let logits: Vec<f32> = output.to_data().to_vec().unwrap();
    assert_eq!(logits.len(), NUM_CLASSES);

    let probs = softmax(&logits);

    let (top_idx, &top_prob) = probs
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .unwrap();

    let is_landmark = is_landmark_index(top_idx) && top_prob >= CONFIDENCE_THRESHOLD;

    let category = if is_landmark {
        landmark_name(top_idx).to_string()
    } else {
        format!("scene_{top_idx}")
    };

    SceneClassification {
        category,
        confidence: top_prob,
        is_landmark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_output_shape() {
        let img = image::DynamicImage::new_rgb8(100, 50);
        let device = Default::default();
        let tensor = preprocess(&img, &device);
        let dims = tensor.dims();
        assert_eq!(dims, [1, 3, INPUT_SIZE, INPUT_SIZE]);
    }

    #[test]
    fn preprocess_values_are_imagenet_normalized() {
        let mut img = image::RgbImage::new(32, 32);
        // Set pixel (0,0) to a known value
        img.put_pixel(0, 0, image::Rgb([128, 64, 255]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let device = Default::default();
        let tensor = preprocess(&dyn_img, &device);
        let data: Vec<f32> = tensor.to_data().to_vec().unwrap();

        let channel_size = INPUT_SIZE * INPUT_SIZE;
        // After resize the exact pixel values may shift, but the range should be
        // approximately in ImageNet normalized range (roughly -2.5 to +2.5).
        // Check that values are NOT in raw [0, 255] range.
        assert!(data[0].abs() < 5.0, "R channel should be normalized");
        assert!(data[channel_size].abs() < 5.0, "G channel should be normalized");
        assert!(data[2 * channel_size].abs() < 5.0, "B channel should be normalized");
    }

    #[test]
    fn landmark_set_contains_expected_entries() {
        assert!(is_landmark_index(334)); // tower
        assert!(is_landmark_index(252)); // palace
        assert!(is_landmark_index(84));  // castle
        assert!(is_landmark_index(355)); // waterfall
    }

    #[test]
    fn is_landmark_returns_false_for_non_landmarks() {
        // Index 0 = airfield, not a landmark
        assert!(!is_landmark_index(0));
        // Index 50 = some non-landmark scene
        assert!(!is_landmark_index(50));
    }

    #[test]
    fn classify_blank_image_does_not_panic() {
        let img = image::DynamicImage::new_rgb8(320, 240);
        let detector = LandmarkDetector::new();
        let result = detector.classify(&img);
        // Should produce some result without panicking
        assert!(result.confidence >= 0.0);
        assert!(result.confidence <= 1.0);
    }
}
