---
name: Use just commands
description: Always use just commands (just build, just test, etc.) instead of raw cargo commands
type: feedback
---

Always use `just` commands instead of raw `cargo` commands.

**Why:** The user wants all command-line invocations captured in the Justfile, and wants consistent usage of the Justfile recipes.

**How to apply:** Use `just build` instead of `cargo build`, `just test` instead of `cargo test`, `just clippy` instead of `cargo clippy`, etc.
