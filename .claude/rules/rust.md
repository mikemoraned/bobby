---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust Rules

- Follow [Rust doc guidelines](https://doc.rust-lang.org/stable/rustdoc/write-documentation/what-to-include.html) if comments are needed
- Use external crates for core things (datetimes, etc); don't write our own
- When multiple crates share a dependency, pull it to workspace-level `[workspace.dependencies]`
- Always use latest stable Rust version and edition; do not use nightly
  - Specify version in `rust-toolchain.toml` and edition in `Cargo.toml`
- `unwrap()` is denied by default. If absolutely needed, annotate with `#[allow(clippy::unwrap_used)]` and give a justification
- Always run `just clippy` after completing each task
- Follow the [NewType](https://doc.rust-lang.org/rust-by-example/generics/new_types.html) idiom — avoid bare Strings or f32s
- Use typed representations instead of untyped arrays (e.g. `DynamicImage` not `Vec<u8>` for images)
- Use `Option::None` when missing data is expected/valid; use `Result::Err` when it represents an invalid state (caller should use `?`)
- Errors: use structured enums with [thiserror](https://docs.rs/thiserror/latest/thiserror/)
- Module structure: different kinds of things (schemas, layers) belong in their own module with their own tests
- Cross-crate models go in a `shared` crate's `lib.rs`
- Keep shared/library types as pure data types — don't add policy or business-logic methods to them. Policy logic belongs in the crate that owns the decision. Only inherent behaviour (formatting, parsing, construction) belongs on the type itself.
- Testing:
  - Core functionality gets inline unit tests
  - Multi-part integration gets integ tests (use captured real data)
  - Prefer high-level invariant-based tests over bespoke examples (consider [quickcheck](https://docs.rs/quickcheck/latest/quickcheck/))
- CLI apps: all config via named CLI params (`--long-form VALUE`); no env vars except `RUST_LOG`
