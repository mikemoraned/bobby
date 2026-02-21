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

# Download Places365 ResNet18 PyTorch weights
download-places-model:
    mkdir -p models
    curl -L -o models/resnet18_places365.pth.tar "http://places2.csail.mit.edu/models_places365/resnet18_places365.pth.tar"
    curl -L -o models/categories_places365.txt "https://raw.githubusercontent.com/CSAILVision/places365/master/categories_places365.txt"

# Convert Places365 model from PyTorch to ONNX (requires: uv add torch torchvision onnx)
convert-places-model:
    cd scripts && uv run python convert_places365.py

# Download the ocrs text detection model
download-text-detection-model:
    mkdir -p models
    curl -L -o models/text-detection.rten \
      "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten"

# Open the top 10 highest-scored annotated candidate images in Preview
top:
    open $(sqlite3 candidates/candidates.db "SELECT annotated_path FROM candidates ORDER BY score_overall DESC LIMIT 10;")
