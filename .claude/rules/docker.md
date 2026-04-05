---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Base images

Two shared multi-arch (arm64 + amd64) base images pinned to bookworm:
- **`Dockerfile.bobby-chef`**: cargo-chef planner + cook with all workspace deps pre-compiled
- **`Dockerfile.bobby-runner`**: debian bookworm-slim + ca-certificates

Architecture-specific RUSTFLAGS are in `.cargo/config.toml`, not Dockerfiles.

## Adding a new service

Use `Dockerfile.service.tmpl` as a starting point — replace `<BIN_NAME>` and add `build-<name>`/`push-<name>` targets to `just/container.just`.

## Conventions

- Shared build tooling changes go in the base images, not service Dockerfiles
- `.dockerignore` excludes `target/`, `store/`, `logs/`, and other large dirs
