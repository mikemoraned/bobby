---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Self-contained Dockerfiles

Each service has its own self-contained Dockerfile with inline cargo-chef stages (planner/builder/runner). No shared base images.

- Base image: `lukemathwalker/cargo-chef:latest-rust-1-bookworm` (planner + builder)
- Runtime: `debian:bookworm-slim` + ca-certificates
- Architecture-specific RUSTFLAGS are in `.cargo/config.toml`, not Dockerfiles
- All `cargo chef cook` and `cargo build` steps use `--mount=type=cache` for registry/git
- `.dockerignore` excludes `target/`, `store/`, `logs/`, and other large dirs

## Platform targets

- **pruner, live-refine**: `linux/arm64` (Hetzner ARM cluster)
- **skeet-feed**: `linux/amd64` (fly.io shared tier)

## Adding a new service

Copy an existing Dockerfile (e.g. `Dockerfile.live-refine`), replace the binary name, and add `build-<name>`/`push-<name>` targets to `just/container.just`.
