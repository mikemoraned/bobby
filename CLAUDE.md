# Bobby

Find selfies people take of themselves with physical landmarks (famous buildings, monuments, places like the Eiffel Tower) — using Bluesky's firehose and modern ML models in Rust. Recreates [the original Twitter-based project](https://github.com/mikemoraned/selfies).

## Prerequisites

```
just prerequisites
```

## Key Commands

- `just prune` — run skeet-prune (listens to firehose, classifies images, saves matches)
- `just clippy` — always run after completing each task
- `just validate-storage` — validate store read/write works

## Methodology

We follow a Walking Skeleton approach: incremental end-to-end slices.

## Reference Docs

Read whichever are relevant before starting work:

- `@docs/architecture.md` — background, target architecture, constraints, technology choices
- `@docs/current-slice.md` — currently active slice and remaining tasks
- `@docs/next-slices.md` — upcoming slices
- `@docs/completed-slices.md` — summary of completed slices

## Invariants / Style

- **Models:** download to `models/` dir; document each in `docs/` (origin, purpose, rationale)
- **Code over comments:** make code self-documenting; add comments only for non-obvious things; substantive docs go in `docs/`
- **Stability:** no `-pre` dependency versions; no direct git dependency versions
  - Exception: `jetstream-oxide` (pre-1.0) is allowed as the best available Rust Jetstream client
- **CLI apps:** all config via named CLI params (e.g. `--long-form VALUE`); no env vars except `RUST_LOG` and secrets (which use `BOBBY_`-prefixed env vars injected via `op run --env-file bobby-local.env` or `bobby-staging.env`)
- **Justfile:** capture all command-line invocations in the Justfile
