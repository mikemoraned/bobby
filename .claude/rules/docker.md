---
paths:
  - "Dockerfile"
  - "Dockerfile.*"
---

# Docker Rules

## Shared target/ cache mount

The shared dependency tree is compiled **once per arch** into a builder-local
`target/` cache mount that every service build reuses — no separate base image to
push. (A pushed multi-arch deps base was tried and rejected: a cooked release
`target/` for the whole workspace × 2 arches is tens of GB and could not be pushed
to GHCR reliably — the push dwarfed the re-cook it was meant to save.)

- Each service build is a single `cargo build --release -p <crate> --bin <bin>`
  with three cache mounts: the cargo registry, the cargo git dir, and `target/`.
- The `target/` mount is **arch-scoped** — `id=bobby-target-arm64` or
  `-amd64`, `sharing=locked`. This is required: `target/release/` artifacts are
  arch-specific but live at the same paths, so an arm64 and an amd64 build sharing
  one mount would link incompatible objects, and concurrent cargo on one target dir
  corrupts it. The id is **hardcoded to match the Dockerfile's pinned platform** (see
  "Platform targets" + the `--platform` in `just/container.just`) rather than derived
  from `${TARGETARCH}` — correct by construction, no reliance on build-arg expansion
  inside a mount id. The registry/git mounts are *not* arch-scoped (crate sources are
  arch-independent, so the download cache is shared across arches).
- Build artifacts live in the **ephemeral** mount, so they are not in the committed
  layer. Copy what the runner needs out to `/build/out/` **inside the same RUN**
  (the binary; for pruner also the `.bpk`/`.rten` model files baked from `target/`),
  then `COPY --from=builder /build/out/...` in the runner stage. `/build/out/` (not
  `/build/<crate>`) avoids colliding with a crate's source dir.
- First build on a cold builder compiles the full dep tree into the mount; every
  later service build reuses it and compiles only its own first-party crates. A
  pruned mount just re-cooks on the next build — never a correctness issue.
- No `cargo chef` — with a shared `target/` mount, chef's separate-deps-layer trick
  adds nothing. `.cargo/config.toml` (rustflags) and `rust-toolchain.toml` (pinned
  toolchain) arrive via `COPY . .` before the single build, so there is no
  cook-vs-build fingerprint mismatch to manage.
- Runtime: `debian:bookworm-slim` + ca-certificates.
- `.dockerignore` excludes `target/`, `store/`, `logs/`, and other large dirs.

## Scope each image to its crate

Each Dockerfile ships exactly one crate. Scope the build so an image doesn't
compile sibling crates:

- Scope to the crate *and* name the shipped binary: `cargo build --release -p <crate> --bin <bin>`. Keep `--bin` even with `-p`, because a bare `-p <crate>` compiles every binary in the crate (e.g. `skeet-prune` has 6, `skeet-refine` has 5) — only the shipped bin(s) should build. Multiple bins from the same crate: repeat `--bin a --bin b`.
- TLS-to-Upstash relies on a `cot` + `deadpool-redis` feature-unification HACK (see root `Cargo.toml`). It survives scoping only because the crate that needs it (`skeet-feed`) declares both deps directly, keeping them in the same scoped subtree. If a new crate needs TLS redis, it must declare `deadpool-redis` directly too.

## Platform targets

- **pruner, live-refine, skeet-publish, optimise, cloudflare-exporter, openai-exporter**: `linux/arm64` (Hetzner ARM cluster)
- **skeet-feed, skeet-appraise**: `linux/amd64` (fly.io shared tier; built emulated on Apple Silicon, ~4× slower than native arm64)

## Adding a new service

Copy an existing service Dockerfile (e.g. `Dockerfile.live-refine`) and set
`-p <crate> --bin <bin>` on the build (see "Scope each image to its crate"). Keep
the three cache mounts and the `/build/out/` copy-out, and set the `target/` mount
`id=bobby-target-<arch>` to match the `--platform` you give it in `just/container.just`.
Add `build-<name>`/`push-<name>` targets to `just/container.just`.
