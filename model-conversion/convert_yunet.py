import onnx
from onnx import version_converter
from pathlib import Path

MODELS_DIR = Path(__file__).parent.parent / "models"
INPUT_MODEL = MODELS_DIR / "face_detection_yunet_2023mar.onnx"
OUTPUT_MODEL = MODELS_DIR / "face_detection_yunet_2023mar_opset16.onnx"
TARGET_OPSET = 16

model = onnx.load(INPUT_MODEL)
print(f"Original opset: {model.opset_import[0].version}")

converted = version_converter.convert_version(model, TARGET_OPSET)
onnx.checker.check_model(converted)
onnx.save(converted, OUTPUT_MODEL)

print(f"Converted to opset: {converted.opset_import[0].version}")
print(f"Saved to: {OUTPUT_MODEL}")
