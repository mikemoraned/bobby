use burn::prelude::*;
use burn::backend::NdArray;

type B = NdArray;

const STRIDES: [usize; 3] = [8, 16, 32];
const FEATURE_MAP_SIZES: [usize; 3] = [80, 40, 20]; // 640/8, 640/16, 640/32

#[derive(Debug, Clone)]
pub struct Detection {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub score: f32,
    pub landmarks: [f32; 10],
}

type ModelOutput = (
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

pub fn decode_and_filter(
    outputs: ModelOutput,
    score_threshold: f32,
    nms_iou_threshold: f32,
) -> Vec<Detection> {
    let (cls_8, cls_16, cls_32, obj_8, obj_16, obj_32, bbox_8, bbox_16, bbox_32, kps_8, kps_16, kps_32) = outputs;

    let cls = [cls_8, cls_16, cls_32];
    let obj = [obj_8, obj_16, obj_32];
    let bbox = [bbox_8, bbox_16, bbox_32];
    let kps = [kps_8, kps_16, kps_32];

    let mut detections = Vec::new();

    for i in 0..3 {
        let stride = STRIDES[i];
        let fm_size = FEATURE_MAP_SIZES[i];

        let cls_data = tensor_to_vec_1d(&cls[i]);
        let obj_data = tensor_to_vec_1d(&obj[i]);
        let bbox_data = tensor_to_vec_2d(&bbox[i], 4);
        let kps_data = tensor_to_vec_2d(&kps[i], 10);

        for row in 0..fm_size {
            for col in 0..fm_size {
                let idx = row * fm_size + col;
                let score = cls_data[idx] * obj_data[idx];

                if score < score_threshold {
                    continue;
                }

                let bbox_row = &bbox_data[idx];
                let cx = (col as f32 + bbox_row[0]) * stride as f32;
                let cy = (row as f32 + bbox_row[1]) * stride as f32;
                let w = bbox_row[2].exp() * stride as f32;
                let h = bbox_row[3].exp() * stride as f32;

                let x = cx - w / 2.0;
                let y = cy - h / 2.0;

                let kps_row = &kps_data[idx];
                let mut landmarks = [0.0f32; 10];
                for k in 0..5 {
                    landmarks[2 * k] = (col as f32 + kps_row[2 * k]) * stride as f32;
                    landmarks[2 * k + 1] = (row as f32 + kps_row[2 * k + 1]) * stride as f32;
                }

                detections.push(Detection {
                    x,
                    y,
                    width: w,
                    height: h,
                    score,
                    landmarks,
                });
            }
        }
    }

    nms(&mut detections, nms_iou_threshold)
}

fn tensor_to_vec_1d(tensor: &Tensor<B, 3>) -> Vec<f32> {
    tensor.to_data().to_vec::<f32>().expect("float tensor")
}

fn tensor_to_vec_2d(tensor: &Tensor<B, 3>, cols: usize) -> Vec<Vec<f32>> {
    let flat: Vec<f32> = tensor.to_data().to_vec::<f32>().expect("float tensor");
    flat.chunks(cols).map(|c| c.to_vec()).collect()
}

fn nms(detections: &mut [Detection], iou_threshold: f32) -> Vec<Detection> {
    detections.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let mut keep = Vec::new();
    let mut suppressed = vec![false; detections.len()];

    for i in 0..detections.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(detections[i].clone());
        for j in (i + 1)..detections.len() {
            if !suppressed[j] && iou(&detections[i], &detections[j]) > iou_threshold {
                suppressed[j] = true;
            }
        }
    }

    keep
}

fn iou(a: &Detection, b: &Detection) -> f32 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.width).min(b.x + b.width);
    let y2 = (a.y + a.height).min(b.y + b.height);

    let intersection = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let area_a = a.width * a.height;
    let area_b = b.width * b.height;
    let union = area_a + area_b - intersection;

    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}
