STORE := "store"
R2_STORE := "s3://hom-bobby/encrypted-store"
FALLBACK_STORE := "fallback-store"

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
    brew install protobuf openssl
    cargo install tokio-console

build:
    cargo build

test:
    cargo test --release

clippy:
    cargo clippy --workspace -- -D warnings

check: build clippy test

classify-examples:
    cargo run --release --bin classify-examples

generate-sse-c-key:
    op item create --vault Dev --title hom-bobby-r2-sse-c-key --category password "password=$(openssl rand -base64 32)" 2>/dev/null \
        || op item edit hom-bobby-r2-sse-c-key --vault Dev "password=$(openssl rand -base64 32)"

validate-storage:
    cargo run --release --bin validate-storage -- --store-path {{ STORE }}

validate-storage-r2:
    op run --env-file bobby.env -- cargo run --release --bin validate-storage -- --store-path {{ R2_STORE }}

OTEL_ENDPOINT := "https://api.honeycomb.io"

find:
    RUST_BACKTRACE=1 cargo run --release --bin finder -- --store-path {{ STORE }}

find-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-finder op run --env-file bobby.env -- cargo run --release --bin finder -- --store-path {{ R2_STORE }} --fallback-local-store {{ FALLBACK_STORE }}

feed:
    RUST_BACKTRACE=1 cargo run --release --bin skeet-feed -- --store-path {{ STORE }}

feed-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-feed op run --env-file bobby.env -- cargo run --release --bin skeet-feed -- --store-path {{ R2_STORE }}

image-metadata-dump image_id:
    cargo run --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

redrive-r2:
    op run --env-file bobby.env -- cargo run --release --bin redrive -- --source-store-path {{ FALLBACK_STORE }} --store-path {{ R2_STORE }}

abort-multipart-uploads:
    op run --env-file bobby.env -- cargo run --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }}

abort-multipart-uploads-confirm:
    op run --env-file bobby.env -- cargo run --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }} --abort

add-to-blocklist at_uri reason="manual":
    cargo run --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"
