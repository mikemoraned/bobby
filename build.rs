use burn_import::onnx::ModelGen;

fn main() {
    ModelGen::new()
        .input("models/yunet.onnx")
        .out_dir("model/")
        .embed_states(true)
        .run_from_script();

    ModelGen::new()
        .input("models/places365.onnx")
        .out_dir("model/")
        .embed_states(true)
        .run_from_script();
}
