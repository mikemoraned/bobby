use burn_import::onnx::ModelGen;

// Build script: panicking with a descriptive message is the idiomatic failure mode, and
// `expect` keeps the precise reason (which env var) that a bare `?` on `VarError` would drop.
#[allow(clippy::expect_used)]
fn main() {
    ModelGen::new()
        .input("../../models/face_detection_yunet_2023mar_opset16.onnx")
        .out_dir("model/")
        .run_from_script();

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    println!(
        "cargo:rustc-env=YUNET_WEIGHTS_PATH={}/model/face_detection_yunet_2023mar_opset16.bpk",
        out_dir
    );
}
