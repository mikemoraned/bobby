---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Shared deps base image

Dependencies are cooked **once** into a pushed multi-arch base image (`bobby-deps:<DEPS_HASH>`) that every service Dockerfile `FROM`s. This dedups the shared dep compile across all services.

- `Dockerfile.deps`: planner + whole-workspace `cargo chef cook` (no `-p` — the base is a superset of all crates' deps), built `--platform linux/arm64,linux/amd64` (target/ is arch-specific) and pushed as `bobby-deps:<DEPS_HASH>`.
- `DEPS_HASH` = md5 of `Cargo.lock` + all `Cargo.toml` + `rust-toolchain.toml` + `.cargo/config.toml`. A stale hash never breaks correctness (cargo re-fingerprints and recompiles the delta in the service build) — it only costs a cache miss.
- `.cargo/config.toml` + `rust-toolchain.toml` are copied before the cook so the base's fingerprints match the service build's; both files are in DEPS_HASH for the same reason. Architecture-specific RUSTFLAGS live in `.cargo/config.toml`, not Dockerfiles.
- Service Dockerfiles: global `ARG DEPS_HASH` → `FROM bobby-deps:${DEPS_HASH}`, then `COPY . .`, `BUILD_GIT_HASH`, and the **scoped** `cargo build -p <crate> --bin <bin>` (the `-p`/`--bin` scoping rule still applies to the build; only the cook is shared).
- All `cargo chef cook` and `cargo build` steps use `--mount=type=cache` for the cargo registry/git dirs.
- Runtime: `debian:bookworm-slim` + ca-certificates.
- `.dockerignore` excludes `target/`, `store/`, `logs/`, and other large dirs.

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

Copy an existing service Dockerfile (e.g. `Dockerfile.live-refine`, which already `FROM`s the deps base) and set `-p <crate> --bin <bin>` on the build (see "Scope each image to its crate"). Add `build-<name>`/`push-<name>` targets to `just/container.just`. If the new crate pulls in deps not already in the base, the next build with a changed `Cargo.toml`/`Cargo.lock` bumps `DEPS_HASH` and rebuilds the base automatically.
