STORE := "store"
R2_STORE := "s3://hom-bobby/encrypted-store"
FALLBACK_STORE := "fallback-store"
OTEL_ENDPOINT := "https://api.honeycomb.io"

import 'just/store.just'
import 'just/feed.just'
import 'just/container.just'
import 'just/cluster.just'

default:
    just --list

download-models:
    mkdir -p models
    curl -L -o models/face_detection_yunet_2023mar.onnx "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"

convert-models: download-models
    cd model-conversion && uv run python convert_yunet.py

prerequisites: convert-models
    brew install protobuf openssl
    cargo install --quiet tokio-console

build:
    cargo build --quiet

test:
    cargo test --quiet --release -p skeet-inspect --features test
    cargo test --quiet --release -p skeet-feed --features test
    cargo test --quiet --release

clippy:
    cargo clippy --quiet --workspace -- -D warnings

check: build clippy test
