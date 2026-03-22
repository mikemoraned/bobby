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

- Always remove dead code — never suppress warnings with `#[allow(dead_code)]` to keep code "for later". Git history is the archive; the working tree should only contain what's currently used.
- `unwrap()` is denied by default. If absolutely needed, annotate with `#[allow(clippy::unwrap_used)]` and give a justification
- Follow the [NewType](https://doc.rust-lang.org/rust-by-example/generics/new_types.html) idiom — avoid bare Strings or f32s
  - When creating a NewType in Rust that is parsed from a `String`, implement the `FromStr` trait, which returns `Result<Self, Self::Err>` where Err is an associated error type you define. 
  - Additionally, provide a `pub fn new(s: impl Into<String>) -> Result<Self, YourError>` constructor for ergonomic direct construction, and have your `FromStr` implementation delegate to `new()`.
  - `FromStr + .parse()` is the Rust community standard for "string → validated domain type" conversions.
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
- Binary layout:
  - All binaries must be named files in `src/bin/` (e.g. `src/bin/finder.rs`), never `src/main.rs` or subdirectories like `src/bin/finder/main.rs`
  - Modules used by binaries live under `src/` and are exposed through `lib.rs`, not placed alongside binaries in `src/bin/`
- CLI apps: all config via named CLI params (`--long-form VALUE`); no env vars except `RUST_LOG`
