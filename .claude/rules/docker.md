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

## Scope each image to its crate

Each Dockerfile ships exactly one crate. Scope both the cook and the build so an image doesn't compile sibling crates:

- `cargo chef prepare` stays workspace-wide — the recipe is deps-only and shared.
- Scope the cook to the crate's dependency subtree: `cargo chef cook --release -p <crate> --recipe-path recipe.json`
- Scope the final build to the crate *and* name the shipped binary: `cargo build --release -p <crate> --bin <bin>`. Keep `--bin` even with `-p`, because a bare `-p <crate>` compiles every binary in the crate (e.g. `skeet-prune` has 6, `skeet-refine` has 5) — only the shipped bin(s) should build. Multiple bins from the same crate: repeat `--bin a --bin b`.
- TLS-to-Upstash relies on a `cot` + `deadpool-redis` feature-unification HACK (see root `Cargo.toml`). It survives scoping only because the crate that needs it (`skeet-feed`) declares both deps directly, keeping them in the same scoped subtree. If a new crate needs TLS redis, it must declare `deadpool-redis` directly too.

## Platform targets

- **pruner, live-refine**: `linux/arm64` (Hetzner ARM cluster)
- **skeet-feed**: `linux/amd64` (fly.io shared tier)

## Adding a new service

Copy an existing Dockerfile (e.g. `Dockerfile.live-refine`), set `-p <crate>` on the cook and `-p <crate> --bin <bin>` on the build (see "Scope each image to its crate"), and add `build-<name>`/`push-<name>` targets to `just/container.just`.
