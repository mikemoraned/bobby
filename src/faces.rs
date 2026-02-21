use burn::backend::NdArray;
use burn::prelude::*;

#[allow(clippy::type_complexity)]
pub mod yunet {
    include!(concat!(env!("OUT_DIR"), "/model/yunet.rs"));
}

type B = NdArray<f32>;

const STRIDES: [usize; 3] = [8, 16, 32];
const CONFIDENCE_THRESHOLD: f32 = 0.7;
const NMS_IOU_THRESHOLD: f32 = 0.3;

#[derive(Debug, Clone)]
pub struct BoundingBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub confidence: f32,
}

#[derive(Debug)]
pub struct FaceDetectionResult {
    pub faces: Vec<BoundingBox>,
    pub image_width: u32,
    pub image_height: u32,
}

pub struct FaceDetector {
    model: yunet::Model<B>,
    device: <B as Backend>::Device,
}

impl FaceDetector {
    pub fn new() -> Self {
        let device = Default::default();
        let model: yunet::Model<B> = Default::default();
        Self { model, device }
    }

    pub fn detect(&self, img: &image::DynamicImage) -> FaceDetectionResult {
        let original_width = img.width();
        let original_height = img.height();

        let input = preprocess(img, &self.device);
        let pad_h = input.dims()[2];
        let pad_w = input.dims()[3];

        let output = self.model.forward(input);
        let faces = postprocess(output, pad_w, pad_h, original_width, original_height);

        FaceDetectionResult {
            faces,
            image_width: original_width,
            image_height: original_height,
        }
    }
}

fn pad_to_multiple_of_32(dim: u32) -> u32 {
    ((dim - 1) / 32 + 1) * 32
}

fn preprocess(img: &image::DynamicImage, device: &<B as Backend>::Device) -> Tensor<B, 4> {
    let width = img.width();
    let height = img.height();
    let pad_w = pad_to_multiple_of_32(width) as usize;
    let pad_h = pad_to_multiple_of_32(height) as usize;

    // YuNet expects BGR float32, no normalization — but we get RGB from the image crate.
    // The model was trained on BGR, so we swap R and B channels.
    let rgb = img.to_rgb8();

    let channel_size = pad_h * pad_w;
    let mut data = vec![0.0f32; 3 * channel_size];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            let pixel_offset = y * pad_w + x;
            // BGR order: channel 0 = B, channel 1 = G, channel 2 = R
            data[pixel_offset] = pixel[2] as f32;
            data[channel_size + pixel_offset] = pixel[1] as f32;
            data[2 * channel_size + pixel_offset] = pixel[0] as f32;
        }
    }

    Tensor::<B, 1>::from_floats(data.as_slice(), device).reshape([1, 3, pad_h, pad_w])
}

#[allow(clippy::type_complexity)]
type YuNetOutput = (
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
    Tensor<B, 3>,
);

fn postprocess(
    output: YuNetOutput,
    pad_w: usize,
    pad_h: usize,
    original_width: u32,
    original_height: u32,
) -> Vec<BoundingBox> {
    let (cls_8, cls_16, cls_32, obj_8, obj_16, obj_32, bbox_8, bbox_16, bbox_32, _kps_8, _kps_16, _kps_32) =
        output;

    let cls_tensors = [cls_8, cls_16, cls_32];
    let obj_tensors = [obj_8, obj_16, obj_32];
    let bbox_tensors = [bbox_8, bbox_16, bbox_32];

    let mut all_boxes: Vec<BoundingBox> = Vec::new();

    for (i, stride) in STRIDES.iter().enumerate() {
        let cols = pad_w / stride;
        let rows = pad_h / stride;

        let cls_data = cls_tensors[i].to_data();
        let obj_data = obj_tensors[i].to_data();
        let bbox_data = bbox_tensors[i].to_data();

        let cls_vals: Vec<f32> = cls_data.to_vec().unwrap();
        let obj_vals: Vec<f32> = obj_data.to_vec().unwrap();
        let bbox_vals: Vec<f32> = bbox_data.to_vec().unwrap();

        for row in 0..rows {
            for col in 0..cols {
                let idx = row * cols + col;

                let cls_score = cls_vals[idx].clamp(0.0, 1.0);
                let obj_score = obj_vals[idx].clamp(0.0, 1.0);
                let confidence = (cls_score * obj_score).sqrt();

                if confidence < CONFIDENCE_THRESHOLD {
                    continue;
                }

                let bbox_offset = idx * 4;
                let dx = bbox_vals[bbox_offset];
                let dy = bbox_vals[bbox_offset + 1];
                let log_w = bbox_vals[bbox_offset + 2];
                let log_h = bbox_vals[bbox_offset + 3];

                let s = *stride as f32;
                let cx = (col as f32 + dx) * s;
                let cy = (row as f32 + dy) * s;
                let w = log_w.exp() * s;
                let h = log_h.exp() * s;

                let x1 = (cx - w / 2.0).clamp(0.0, original_width as f32);
                let y1 = (cy - h / 2.0).clamp(0.0, original_height as f32);
                let x2 = (cx + w / 2.0).clamp(0.0, original_width as f32);
                let y2 = (cy + h / 2.0).clamp(0.0, original_height as f32);

                all_boxes.push(BoundingBox {
                    x1,
                    y1,
                    x2,
                    y2,
                    confidence,
                });
            }
        }
    }

    nms(&mut all_boxes, NMS_IOU_THRESHOLD);
    all_boxes
}

fn iou(a: &BoundingBox, b: &BoundingBox) -> f32 {
    let inter_x1 = a.x1.max(b.x1);
    let inter_y1 = a.y1.max(b.y1);
    let inter_x2 = a.x2.min(b.x2);
    let inter_y2 = a.y2.min(b.y2);

    let inter_area = (inter_x2 - inter_x1).max(0.0) * (inter_y2 - inter_y1).max(0.0);
    let area_a = (a.x2 - a.x1) * (a.y2 - a.y1);
    let area_b = (b.x2 - b.x1) * (b.y2 - b.y1);
    let union_area = area_a + area_b - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

fn nms(boxes: &mut Vec<BoundingBox>, iou_threshold: f32) {
    boxes.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

    let mut keep = vec![true; boxes.len()];
    for i in 0..boxes.len() {
        if !keep[i] {
            continue;
        }
        for j in (i + 1)..boxes.len() {
            if !keep[j] {
                continue;
            }
            if iou(&boxes[i], &boxes[j]) > iou_threshold {
                keep[j] = false;
            }
        }
    }

    let mut idx = 0;
    boxes.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_output_shape_is_padded_to_32() {
        let img = image::DynamicImage::new_rgb8(100, 50);
        let device = Default::default();
        let tensor = preprocess(&img, &device);
        let dims = tensor.dims();
        assert_eq!(dims[0], 1);
        assert_eq!(dims[1], 3);
        assert_eq!(dims[2], 64); // 50 padded to 64
        assert_eq!(dims[3], 128); // 100 padded to 128
    }

    #[test]
    fn preprocess_values_are_raw_pixel_range() {
        let mut img = image::RgbImage::new(32, 32);
        img.put_pixel(0, 0, image::Rgb([128, 64, 255]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let device = Default::default();
        let tensor = preprocess(&dyn_img, &device);
        let data: Vec<f32> = tensor.to_data().to_vec().unwrap();
        // BGR order: channel 0 = B (255), channel 1 = G (64), channel 2 = R (128)
        let channel_size = 32 * 32;
        assert_eq!(data[0], 255.0); // B at pixel (0,0)
        assert_eq!(data[channel_size], 64.0); // G at pixel (0,0)
        assert_eq!(data[2 * channel_size], 128.0); // R at pixel (0,0)
    }

    #[test]
    fn nms_suppresses_overlapping_boxes() {
        let mut boxes = vec![
            BoundingBox {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 10.0,
                confidence: 0.9,
            },
            BoundingBox {
                x1: 1.0,
                y1: 1.0,
                x2: 11.0,
                y2: 11.0,
                confidence: 0.8,
            },
            BoundingBox {
                x1: 100.0,
                y1: 100.0,
                x2: 110.0,
                y2: 110.0,
                confidence: 0.85,
            },
        ];
        nms(&mut boxes, 0.3);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].confidence, 0.9);
        assert_eq!(boxes[1].confidence, 0.85);
    }

    #[test]
    fn no_faces_in_blank_image() {
        let img = image::DynamicImage::new_rgb8(320, 240);
        let detector = FaceDetector::new();
        let result = detector.detect(&img);
        assert!(result.faces.is_empty());
    }
}
