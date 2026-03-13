# YuNet Face Detection Model

## Source

- **Repository**: [opencv/opencv_zoo](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet)
- **File**: `face_detection_yunet_2023mar.onnx`
- **License**: MIT
- **Original opset**: 11 (converted to opset 16 for burn-import compatibility)

## What it does

YuNet is a lightweight face detection model that outputs bounding boxes, confidence scores, and 5 facial landmarks (right eye, left eye, nose, right mouth corner, left mouth corner) for each detected face.

It can detect faces between approximately 10x10 and 300x300 pixels.

## Why we use it

We need to filter Bluesky images to only those containing face-on human faces. YuNet provides both face detection and landmark positions, which allows us to:

1. Detect whether an image contains faces at all
2. Determine if a face is frontal (not side-profile) using landmark geometry
3. Locate which quadrant of the image the face occupies

## Input / Output

- **Input**: `[1, 3, 640, 640]` NCHW BGR float32 tensor (raw pixel values 0-255)
- **Outputs** (12 tensors at 3 stride levels: 8, 16, 32):
  - `cls_{stride}`: classification confidence `[1, N, 1]`
  - `obj_{stride}`: objectness score `[1, N, 1]`
  - `bbox_{stride}`: bounding box offsets `[1, N, 4]`
  - `kps_{stride}`: keypoint offsets `[1, N, 10]` (5 landmarks x 2 coords)

Where N = (640/stride)^2.

## Post-processing

1. Generate anchor grid per stride level
2. Decode boxes: `cx = (col + bbox[0]) * stride`, `w = exp(bbox[2]) * stride`
3. Combine scores: `score = cls * obj`
4. Filter by score threshold (0.6)
5. Non-maximum suppression with IoU threshold (0.3)

## Integration

Run via Burn (ndarray backend). Model weights converted from ONNX to `.bpk` format at compile time by `burn-import`.

Opset conversion from 11 to 16 is handled by `model-conversion/convert_yunet.py`.

## Performance

WIDER Face validation set:
- Easy: 0.8844 AP
- Medium: 0.8656 AP
- Hard: 0.7503 AP
