/// Emit `BUILD_GIT_HASH` as a `rustc-env` build script instruction.
///
/// Reads the `BUILD_GIT_HASH` environment variable set by the build system
/// (e.g. `just`/Docker via `--build-arg`). Falls back to `"unknown"` when
/// not set, so local `cargo build` without the variable always compiles.
pub fn emit_git_hash() {
    let hash = std::env::var("BUILD_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_GIT_HASH={hash}");
}
