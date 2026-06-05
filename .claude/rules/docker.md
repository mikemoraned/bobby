---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Shared builder + `--target`

There are two Dockerfiles, one per platform: **`Dockerfile.cluster`** (arm64,
Hetzner) and **`Dockerfile.fly`** (amd64, fly.io). Each has a **single `builder`
stage** that compiles *every* shipped binary for that platform in one
`cargo build`, then one thin `runner-<name>` stage per service that copies out only
its own binary. A given service image is produced with
`docker buildx build -f Dockerfile.<cluster|fly> --target runner-<name>`.

One builder, not per-service Dockerfiles, so the dep tree compiles once per platform:
BuildKit reuses the cached `builder` across `--target` invocations.

### Builder stage

- One `cargo build --release` listing every shipped crate/bin for the platform:
  `-p <crate> --bin <bin>` repeated (e.g. `cloudflare-exporter` ships two bins, so
  `--bin sync_operations --bin sync_storage`). Keep `--bin` — a bare `-p` builds
  every bin in the crate (`skeet-prune` has 6, `skeet-refine` has 5).
- Three cache mounts: cargo registry, cargo git, and an **arch-scoped** `target/`
  (`id=bobby-target-arm64` / `-amd64`, `sharing=locked`). The `target/` mount keeps
  cargo incremental: a source-only edit re-runs the builder RUN (BuildKit invalidates
  it because `COPY . .` changed), but cargo reuses every dep from the mount and
  recompiles only the changed first-party crate(s). The id is hardcoded to the
  Dockerfile's platform — arm64 and amd64 artifacts share `target/release/` paths and
  must never share one mount.
- Build artifacts live in the ephemeral mount, so copy what the runners need out to
  `/build/out/` **inside the RUN** (every binary; for the cluster builder also the
  `.bpk`/`.rten` files baked from `target/` for pruner). `dash` (the default RUN
  shell) has no brace expansion — use space-separated `cp` args.
- `ARG BUILD_GIT_HASH` + `ENV BUILD_GIT_HASH` before the build. It applies to every
  crate in the build (most call `emit_git_hash` in build.rs); the build-arg is passed
  by every `container.just` recipe. Changing commit re-runs the builder and
  recompiles only the leaf bins that bake the hash — deps stay cached.
- `.cargo/config.toml` (rustflags) and `rust-toolchain.toml` (pinned toolchain)
  arrive via `COPY . .` before the single build; no `cargo chef`.

### Runner stages

- `runner-base` (`debian:bookworm-slim` + ca-certificates) once; each
  `runner-<name>` is `FROM runner-base` and just `COPY --from=builder /build/out/<bin>`
  plus any config/`CMD`. These thin stages are the pushed images.
- TLS-to-Upstash relies on a `cot` + `deadpool-redis` feature-unification HACK (see
  root `Cargo.toml`); it holds because `skeet-feed` declares both deps directly.

## Platform targets

- **`Dockerfile.cluster`** (`linux/arm64`, Hetzner): pruner, live-refine, skeet-publish, optimise, cloudflare-exporter, openai-exporter.
- **`Dockerfile.fly`** (`linux/amd64`, fly.io shared tier; built emulated on Apple Silicon, ~4× slower than native arm64): skeet-feed, skeet-appraise.

## Adding a new service

Add a `-p <crate> --bin <bin>` to the relevant builder's `cargo build`, a `cp` of its
binary into `/build/out/`, and a `runner-<name>` stage that copies it out. Add
`build-<name>`/`push-<name>` recipes to `just/container.just` pointing at
`-f Dockerfile.<cluster|fly> --target runner-<name>`. A new crate needing TLS redis
must declare `deadpool-redis` directly (see the HACK above).
