STORE := "store"

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
    cargo test --release

clippy:
    cargo clippy --workspace -- -D warnings

check: build clippy test

classify-examples:
    cargo run --release --bin classify-examples

validate-storage:
    cargo run --release --bin validate-storage -- --store-path {{ STORE }}

find:
    cargo run --release --bin finder -- --store-path {{ STORE }}

feed:
    cargo run --release --bin skeet-feed -- --store-path {{ STORE }}

image-metadata-dump image_id:
    cargo run --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

add-to-blocklist at_uri reason="manual":
    cargo run --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"
