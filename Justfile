# Run the firehose listener
run:
    cargo run --release

# Build the project
build:
    cargo build

# Run tests
test:
    cargo test

# Run clippy
clippy:
    cargo clippy

# Run tests and clippy
check: test clippy

# Download the YuNet face detection ONNX model (~227KB)
download-model:
    mkdir -p models
    curl -L -o models/yunet.onnx "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
