use burn_import::onnx::ModelGen;

fn main() {
    // `embed_states(true)` bakes the weights into the binary via `include_bytes!`, so the
    // generated model loads from `.rodata` at runtime with no external file to locate.
    ModelGen::new()
        .input("../../models/face_detection_yunet_2023mar_opset16.onnx")
        .out_dir("model/")
        .embed_states(true)
        .run_from_script();
}
