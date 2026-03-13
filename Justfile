default:
    just --list

download-models:
    mkdir -p models
    curl -L -o models/face_detection_yunet_2023mar.onnx "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"

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

find:
    cargo run --release --bin skeet-finder -- --store-path store

feed:
    cargo run --release --bin skeet-feed -- --store-path store
