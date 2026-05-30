---
name: Use just test for full test suite
description: Always use `just test` to run the full test suite, not `cargo test --workspace` or `cargo nextest run` directly
type: feedback
---

Always use `just test` to run the full test suite. Do not use `cargo test --workspace` or invoke `cargo nextest run` directly.

**Why:** The Justfile captures the canonical invocation, which includes `--release --features integ` — running nextest without those flags misses the integration-test features and may silently change behaviour. The Justfile is the single source of truth for command-line invocations in this project (also stated in `CLAUDE.md`).

**How to apply:** When running the full test suite, use `just test`. When Docker is unavailable (e.g. sandboxed runs), use `just test-no-docker`.
