default:
    just --list

download-models:
    mkdir -p models
    curl -L -o models/face_detection_yunet_2023mar.onnx "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
    curl -L -o models/text-detection.rten "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten"
    curl -L -o models/text-recognition.rten "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten"

convert-models: download-models
    cd model-conversion && uv run python convert_yunet.py

prerequisites: convert-models
    brew install protobuf

build:
    cargo build

test:
    cargo test

clippy:
    cargo clippy --workspace -- -D warnings

check: build clippy test

classify-examples:
    cargo run --release --bin classify-examples

find:
    cargo run --release --bin skeet-finder -- --store-path store

feed:
    cargo run --release --bin skeet-feed -- --store-path store
