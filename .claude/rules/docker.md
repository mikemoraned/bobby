---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Shared builder + `--target`

There are two Dockerfiles, one per service-set: **`Dockerfile.cluster`** (Hetzner)
and **`Dockerfile.fly`** (fly.io). Both build `linux/amd64`. Each has a **single
`builder` stage** that compiles *every* shipped binary for that set in one
`cargo build`, then one thin `runner-<name>` stage per service that copies out only
its own binary. A given service image is produced with
`docker buildx build -f Dockerfile.<cluster|fly> --target runner-<name>`.

One builder, not per-service Dockerfiles, so the dep tree compiles once per set:
BuildKit reuses the cached `builder` across `--target` invocations.

### Builder stage

- One `cargo build --release` listing every shipped crate/bin for the platform:
  `-p <crate> --bin <bin>` repeated (e.g. `cloudflare-exporter` ships two bins, so
  `--bin sync_operations --bin sync_storage`). Keep `--bin` — a bare `-p` builds
  every bin in the crate (`skeet-prune` has 6, `skeet-refine` has 5).
- Three cache mounts: cargo registry, cargo git, and `target/`, **all three shared by
  both Dockerfiles** (`id=bobby-target-amd64` for the target mount, `sharing=shared`).
  The `target/` mount keeps cargo incremental: a source-only edit re-runs the builder
  RUN (BuildKit invalidates it because `COPY . .` changed), but cargo reuses every dep
  from the mount and recompiles only the changed first-party crate(s). Both sets build
  `linux/amd64` with the same toolchain, rustflags, and `release` profile, so their
  artifacts are interchangeable — sharing one mount lets common deps compile once across
  both sets. Cargo's target dir is fingerprint-keyed, so the two sets' disjoint bins
  coexist safely.
- **Use `sharing=shared`, never `locked`/`private`.** These are `--push` builds run
  back-to-back by `cluster-deploy-all` (or similar); pushing an emulated-amd64 image is slow, so the previous build still holds the mount ref when the next starts. With `locked` (or
  `private`) BuildKit hands the next build a *second, empty* mount instance rather than
  waiting — the instances then ping-pong and every other build recompiles from scratch
  (observed: `runner-skeet-publish`/`runner-cloudflare-exporter` doing full 839-crate
  cold builds while the interleaved targets were fully cached). `shared` keeps a single
  instance every build reuses. It's safe: `just` runs the builds sequentially, and even
  concurrent cargo processes serialise via cargo's own `.cargo-lock` on the target dir.
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
- Base image is the official `rust:1.94-bookworm`, with its tag kept equal to
  `rust-toolchain.toml`'s `channel`. `rust-toolchain.toml` stays the source of truth;
  matching the image tag to it means rustup finds the pinned toolchain already
  installed and skips the ~243s "syncing channel updates" re-download at build time.
  (Bookworm matches the `debian:bookworm-slim` runner's glibc.)

### Runner stages

- `runner-base` (`debian:bookworm-slim` + ca-certificates) once; each
  `runner-<name>` is `FROM runner-base` and just `COPY --from=builder /build/out/<bin>`
  plus any config/`CMD`. These thin stages are the pushed images.
- TLS-to-Upstash relies on a `cot` + `deadpool-redis` feature-unification HACK (see
  root `Cargo.toml`); it holds because `skeet-feed` declares both deps directly.

## Platform targets

- **`Dockerfile.cluster`** (`linux/amd64`, Hetzner): pruner, live-refine, skeet-publish, optimise, cloudflare-exporter, openai-exporter.
- **`Dockerfile.fly`** (`linux/amd64`, fly.io shared tier): skeet-feed, skeet-appraise.

Both are `linux/amd64`, built emulated on Apple Silicon (~4× slower than native).

## Adding a new service

Add a `-p <crate> --bin <bin>` to the relevant builder's `cargo build`, a `cp` of its
binary into `/build/out/`, and a `runner-<name>` stage that copies it out. Add
`build-<name>`/`push-<name>` recipes to `just/container.just` pointing at
`-f Dockerfile.<cluster|fly> --target runner-<name>`. A new crate needing TLS redis
must declare `deadpool-redis` directly (see the HACK above).
