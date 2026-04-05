---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Base image architecture

Two shared base images centralise all common setup:

- **`Dockerfile.bobby-chef`** (build base): cargo-chef planner + cook stage — produces an image with all workspace dependencies pre-compiled. Includes protobuf-compiler and RUSTFLAGS targeting Neoverse N1 (Hetzner CAX ARM).
- **`Dockerfile.bobby-runner`** (runtime base): debian bookworm-slim + ca-certificates

Service Dockerfiles (`Dockerfile.pruner`, `Dockerfile.skeet-feed`, `Dockerfile.live-refine`) inherit from these via `BOBBY_CHEF` and `BOBBY_RUNNER` ARGs. They only need to COPY source and run `cargo build --release --bin <name>`.

## Build flow

1. `just build-base` — builds both base images locally (must be re-run when deps change)
2. `just push-base` — pushes them to ghcr
3. `just build-<service>` — builds a service image on top of the base

## Adding a new service Dockerfile

1. Declare `ARG BOBBY_CHEF` and `ARG BOBBY_RUNNER` at the top with ghcr defaults
2. `FROM ${BOBBY_CHEF} AS builder` — COPY source, `cargo build --release --bin <name>`
3. `FROM ${BOBBY_RUNNER} AS runner` — COPY binary from builder
4. Add `--mount=type=cache` for cargo registry and git on the `cargo build` step
5. Add `build-<name>` / `push-<name>` targets to `just/container.just`

## Conventions

- All shared build tooling changes (protobuf, RUSTFLAGS, cargo-chef recipe/cook) go in the base images, not in service Dockerfiles
- `models/` directory is only COPY'd in service builder stages — the base image only needs Cargo.toml/Cargo.lock via cargo-chef
