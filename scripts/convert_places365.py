"""Convert Places365 ResNet18 PyTorch weights to ONNX format.

Requirements: pip install torch torchvision onnx

Downloads are handled separately via `just download-places-model`.
This script expects:
  - models/resnet18_places365.pth.tar  (PyTorch checkpoint)
It produces:
  - models/places365.onnx
"""

import torch
import torchvision.models as models

CHECKPOINT_PATH = "../models/resnet18_places365.pth.tar"
OUTPUT_PATH = "../models/places365.onnx"
NUM_CLASSES = 365


def main():
    # Create ResNet18 with 365 output classes
    model = models.resnet18(num_classes=NUM_CLASSES)

    # Load the Places365 checkpoint
    checkpoint = torch.load(CHECKPOINT_PATH, map_location="cpu", weights_only=False)
    state_dict = checkpoint.get("state_dict", checkpoint)

    # The Places365 checkpoint prefixes keys with "module." — strip that
    new_state_dict = {}
    for key, value in state_dict.items():
        new_key = key.replace("module.", "")
        new_state_dict[new_key] = value

    model.load_state_dict(new_state_dict)
    model.eval()

    # Export to ONNX
    dummy_input = torch.randn(1, 3, 224, 224)
    torch.onnx.export(
        model,
        dummy_input,
        OUTPUT_PATH,
        opset_version=16,
        input_names=["input"],
        output_names=["output"],
        dynamic_axes=None,
    )

    print(f"Exported ONNX model to {OUTPUT_PATH}")


if __name__ == "__main__":
    main()
