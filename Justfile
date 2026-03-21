STORE := "store"
R2_STORE := "s3://hom-bobby/encrypted-store"

# Read a 1Password item's password by name, picking the most recently created if duplicates exist
_op-read-latest vault item:
    @op item list --vault {{ vault }} --format json \
        | jq -r '[.[] | select(.title == "{{ item }}")] | sort_by(.created_at) | last | .id' \
        | xargs -I{} op read "op://{{ vault }}/{}/password"

_r2-args:
    @echo "--store-path {{ R2_STORE }} --s3-endpoint $(just _op-read-latest Dev hom-bobby-r2-local-rw-endpoint) --s3-access-key-id $(just _op-read-latest Dev hom-bobby-r2-local-rw-id) --s3-secret-access-key $(just _op-read-latest Dev hom-bobby-r2-local-rw-key) --sse-c-key $(just _op-read-latest Dev hom-bobby-r2-sse-c-key)"

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
    cargo run --release --bin validate-storage -- $(just _r2-args)

OTEL_ENDPOINT := "https://api.honeycomb.io"

_otel-key:
    @just _op-read-latest Dev hom-bobby-hcoltp-local-ingest

find:
    RUST_BACKTRACE=1 cargo run --release --bin finder -- --store-path {{ STORE }}

find-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=$(just _otel-key)" OTEL_SERVICE_NAME=skeet-finder cargo run --release --bin finder -- $(just _r2-args)

feed:
    RUST_BACKTRACE=1 cargo run --release --bin skeet-feed -- --store-path {{ STORE }}

feed-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=$(just _otel-key)" OTEL_SERVICE_NAME=skeet-feed cargo run --release --bin skeet-feed -- $(just _r2-args)

image-metadata-dump image_id:
    cargo run --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

add-to-blocklist at_uri reason="manual":
    cargo run --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"
