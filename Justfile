STORE := "store"
R2_STORE := "s3://hom-bobby/encrypted-store"
FALLBACK_STORE := "fallback-store"
OTEL_ENDPOINT := "https://api.honeycomb.io"

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

classify-examples:
    cargo run --quiet --release --bin classify-examples

generate-sse-c-key:
    op item create --vault Dev --title hom-bobby-r2-sse-c-key --category password "password=$(openssl rand -base64 32)" 2>/dev/null \
        || op item edit hom-bobby-r2-sse-c-key --vault Dev "password=$(openssl rand -base64 32)"

validate-storage:
    cargo run --quiet --release --bin validate-storage -- --store-path {{ STORE }}

validate-storage-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin validate-storage -- --store-path {{ R2_STORE }}

prune:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin pruner -- --store-path {{ STORE }}

prune-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-prune op run --env-file bobby.env -- cargo run --quiet --release --bin pruner -- --store-path {{ R2_STORE }} --fallback-local-store {{ FALLBACK_STORE }}

inspect:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin skeet-inspect -- --store-path {{ STORE }}

inspect-fallback:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin skeet-inspect -- --store-path {{ FALLBACK_STORE }}

inspect-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-inspect op run --env-file bobby.env -- cargo run --quiet --release --bin skeet-inspect -- --store-path {{ R2_STORE }}

export-image image_id:
    cargo run --quiet --release --bin export-image -- --store-path {{ STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

export-image-r2 image_id:
    cargo run --quiet --release --bin export-image -- --store-path {{ R2_STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

image-metadata-dump image_id:
    cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

image-metadata-dump-r2 image_id:
    op run --env-file bobby.env -- cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ R2_STORE }} --image-id {{ image_id }}

image-metadata-dump-fallback image_id:
    cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ FALLBACK_STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --quiet --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

redrive-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin redrive -- --source-store-path {{ FALLBACK_STORE }} --store-path {{ R2_STORE }} --mode upload-and-delete

redrive-local-to-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin redrive -- --source-store-path {{ STORE }} --store-path {{ R2_STORE }} --mode upload --most-recent-first

abort-multipart-uploads:
    op run --env-file bobby.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }}

abort-multipart-uploads-confirm:
    op run --env-file bobby.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }} --abort

summarise:
    cargo run --quiet --release --bin summarise -- --store-path {{ STORE }}

summarise-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin summarise -- --store-path {{ R2_STORE }}

compact:
    cargo run --quiet --release --bin compact -- --store-path {{ STORE }}

compact-fallback:
    cargo run --quiet --release --bin compact -- --store-path {{ FALLBACK_STORE }}

compact-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin compact -- --store-path {{ R2_STORE }}

add-to-blocklist at_uri reason="manual":
    cargo run --quiet --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"

train:
    op run --env-file bobby.env -- cargo run --quiet --release --bin train

refine:
    op run --env-file bobby.env -- cargo run --quiet --release --bin refine -- --store-path {{ STORE }}

refine-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin refine -- --store-path {{ R2_STORE }}

live-refine:
    op run --env-file bobby.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ STORE }}

live-refine-r2:
    op run --env-file bobby.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

# --- Feed ---

STAGING_HOSTNAME := "bobby-staging.houseofmoran.io"
PUBLISHER_DID := "did:plc:cjvdzmk4iapi5p5orrasehxp"

feed:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin skeet-feed -- --store-path {{ STORE }} --hostname localhost:8080 --publisher-did did:web:localhost:8080

feed-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-feed op run --env-file bobby.env -- cargo run --quiet --release --bin skeet-feed -- --store-path {{ R2_STORE }} --hostname {{ STAGING_HOSTNAME }} --publisher-did {{ PUBLISHER_DID }}

test_feed:
    cargo test --quiet --release -p skeet-feed --features test

deploy_staging: deploy_staging_secrets deploy_staging_app test_staging

deploy_staging_secrets:
    op inject -i bobby.env | grep "^BOBBY_" | fly secrets import --config fly.staging.toml

deploy_staging_app:
    fly deploy --config fly.staging.toml

test_staging:
    TEST_BASE_URL=https://bobby-staging.fly.dev cargo test --quiet --release -p skeet-feed --features test

register-feed:
    op run --env-file register.env -- cargo run --quiet --release --bin register-feed -- --hostname {{ STAGING_HOSTNAME }}
