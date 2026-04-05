# Current Slice: Slice 11 — Improve Rust compile times, both locally and remotely

Reduce compile times by removing unused dependencies, disabling unnecessary default features, and unifying feature resolution across the workspace.

### Tasks

#### Unused dependency audit
- [x] Install `cargo-machete` (`cargo install cargo-machete`) and run `cargo machete --with-metadata` across the workspace; remove confirmed unused deps
- [x] Add false-positive ignores to root `Cargo.toml` for deps only used via macros or `build.rs` (e.g. proc-macro crates, codegen deps)

#### Feature pruning
- [x] Run `cargo tree --edges features` on the workspace to see which features are activated for heavy deps (e.g. `tokio`, `serde`, `reqwest`)
- [x] Install `cargo-features-manager` (`cargo install cargo-features-manager`) and run `cargo features prune`; review suggestions per crate
- [x] For high-impact deps, switch to `default-features = false` and explicitly enable only needed features; verify with `cargo check --all-targets`
- [x] Centralise shared dependency versions and feature selections in `[workspace.dependencies]` if not already done

#### cargo-chef dependency caching
- [x] Restructure both Dockerfiles into three stages: `planner` (runs `cargo chef prepare`), `builder` (runs `cargo chef cook --release` then `cargo build --release`), and `runner` (copies the binary); use `lukemathwalker/cargo-chef:latest-rust-1` as the base and install `protobuf-compiler` in a shared `chef` stage
- [x] For `Dockerfile.pruner`, ensure the `models/` directory is only `COPY`'d into the `builder` stage (after `cargo chef cook`), not the `planner` stage — cargo-chef only needs `Cargo.toml`/`Cargo.lock` files, not build-time assets
- [x] Test that a source-only change (no new deps) results in a cache hit on the `cargo chef cook` layer by building twice and checking for `CACHED` in the build output

#### BuildKit cache mounts
- [x] Ensure BuildKit is enabled: set `DOCKER_BUILDKIT=1` in the environment or use `docker buildx build` instead of `docker build`
- [x] Add `--mount=type=cache,target=/usr/local/cargo/registry,sharing=locked` and `--mount=type=cache,target=/usr/local/cargo/git,sharing=locked` to the `RUN` steps for both `cargo chef cook` and `cargo build --release`

#### Shared base docker image

We are getting good cache usage across *different* Dockerfiles by accident, as the `cargo chef` recipies are identical. This is fragile. Also, a lot of our other instructions are also very similar.

- [x] try creating a base `bobby` base image (`Dockerfile.bobby`) which the various other `Dockerfile`s can inherit from by sharing the same base image. This base image can/should be published explicitly to ghcr. This should allow us a to centralise all shared setup.

#### Make fly.io deploy consistent

* [x] we should update the fly.io deployment so that it uses the same setup and we don't get fly.io to rebuild anything i.e. we use ghcr for fly.io as well

Summary of changes:
- `fly.staging.toml`: switched from `[build] dockerfile` to `[build] image` pointing at pre-built ghcr image. Fly.io shared tier doesn't support ARM, so skeet-feed stays on `shared-cpu-1x` (amd64).
- `bobby-chef` is local-only (5GB, too large for ghcr) — built per-platform with arch tags (`bobby-chef:arm64`, `bobby-chef:amd64`). Service builds depend on the correct arch variant automatically.
- `bobby-runner` (~106MB) pushed to ghcr as multi-arch (`linux/arm64,linux/amd64`) via `bobby-multiarch` buildx builder.
- Moved architecture-specific RUSTFLAGS (`-C target-cpu=neoverse-n1`) from Dockerfiles into `.cargo/config.toml` under `[target.aarch64-unknown-linux-gnu]` — applies automatically per platform.
- `deploy_staging_app` now depends on `push-skeet-feed` so the image is always fresh before `fly deploy`.
- Added OCI source labels to all Dockerfiles for GitHub package linking.
- ghcr packages set to public so fly.io can pull without registry auth.
- Pinned cargo-chef base to `bookworm` to match runner's glibc.
