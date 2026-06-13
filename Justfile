STORE := "store"
R2_STORE := "s3://hom-bobby/encrypted-store"

import 'just/store.just'
import 'just/refine.just'
import 'just/feed.just'
import 'just/appraise.just'
import 'just/publish.just'
import 'just/container.just'
import 'just/cluster.just'
import 'just/cloudflare.just'
import 'just/openai.just'
import 'just/local.just'

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
    brew install protobuf openssl cargo-nextest gettext
    cargo install --locked cargo-mutants

build:
    cargo build --quiet

test:
    cargo nextest run --release --features integ

# Omits tests marked _docker; safe to run without Docker
test-no-docker:
    cargo nextest run --release --features integ --profile no-docker

end_to_end_test: end_to_end_test_cloudflare end_to_end_test_openai end_to_end_test_feed_staging end_to_end_test_appraise_staging

mutants-on-diff:
    git diff main > /tmp/bobby-mutants-diff.patch
    cargo mutants --in-diff /tmp/bobby-mutants-diff.patch

clippy:
    cargo clippy --quiet --workspace -- -D warnings

check: build clippy test
