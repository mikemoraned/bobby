# Bobby

Find selfies people take of themselves with physical landmarks (famous buildings, monuments, places like the Eiffel Tower) — using Bluesky's firehose and modern ML models in Rust. Recreates [the original Twitter-based project](https://github.com/mikemoraned/selfies).

## Prerequisites

```
just prerequisites
```

## Key Commands

- `just prune` — run skeet-prune (listens to firehose, classifies images, saves matches)
- `just clippy` — always run after completing each task
- `just test` — run all unit + integration tests (integration tests spin up local dependencies via testcontainers; requires Docker)
  - **Claude:** you run in a sandbox without Docker — use `just test-no-docker` instead
- `just test-no-docker` — same as `just test` but omits testcontainers-based tests; safe to run without Docker
- `just end_to_end_test` — run tests that require live external APIs (OpenAI, Cloudflare, staging)
- `just mutants-on-diff` — run mutation testing on changed code; run after completing any non-trivial change
- `just validate-storage` — validate store read/write works

## Methodology

We follow a Walking Skeleton approach: incremental end-to-end slices.

### Test-Driven Development

When doing TDD, always keep the code compiling at every step:
1. Write a stub that compiles but returns a wrong/trivial value (e.g. `0`, `false`, `""`)
2. Write tests asserting the correct behaviour — they should **fail** (wrong value, not compile error)
3. Implement correctly — tests should now pass

## Reference Docs

Read whichever are relevant before starting work:

- `@docs/architecture.md` — background, target architecture, constraints, technology choices
- `@docs/current-slice.md` — currently active slice and remaining tasks
- `@docs/next-slices.md` — upcoming slices
- `@docs/completed-slices.md` — summary of completed slices

## Shell Commands

Never re-issue a build or test command if one is already running or has just completed. If a command is taking a while, wait for it rather than spawning duplicates.

## Security

Never generate shell commands that capture secrets in a variable — always use `op run --env-file` patterns so credentials flow through the process environment without touching the LLM context.

## Invariants / Style

- **Models:** download to `models/` dir; document each in `docs/` (origin, purpose, rationale)
- **Code over comments:** make code self-documenting; add comments only for non-obvious things; substantive docs go in `docs/`
- **Stability:** no `-pre` dependency versions; no direct git dependency versions
  - Exception: `jetstream-oxide` (pre-1.0) is allowed as the best available Rust Jetstream client
- **CLI apps:** all config via named CLI params (e.g. `--long-form VALUE`); no env vars except `RUST_LOG` and secrets (which use `BOBBY_`-prefixed env vars injected via `op run --env-file bobby-local.env` or a per-service staging file like `bobby-feed-staging.env`)
- **Justfile:** capture all command-line invocations in the Justfile
