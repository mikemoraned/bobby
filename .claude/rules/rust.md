---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust Rules

- Follow [Rust doc guidelines](https://doc.rust-lang.org/stable/rustdoc/write-documentation/what-to-include.html) if comments are needed

- **Prefer adding a battle-tested dependency over hand-rolling non-trivial logic.** Datetimes, RNG, statistics, ML metrics, parsers — if a published crate solves it, take the dep.
  - Default to the dep even when the standard-library or hand-rolled version "looks short". Bespoke code accumulates: it needs tests, edge-case handling, and ongoing maintenance.
  - Before claiming a library doesn't support what you need, check the *latest* version's docs (the API may have changed since older releases). State the version you checked.
  - Hand-rolling is acceptable only for genuinely application-specific logic (e.g. domain enums, business rules) — not for replicable algorithms like ROC-AUC, train/test split, or seeded shuffling.
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
  - Functions that can fail must return `Result<T, E>`, never `bool` for success/failure
  - Enums used as return types must only contain success variants; failure cases belong in the `Err` side of a `Result`. For example, a `verify()` function should return `Result<VerifyResult, E>` where `VerifyResult` has `Match`/`NotFound`/`Mismatch` (all valid outcomes) — not a `Failed` variant baked into the enum
- Module structure: different kinds of things (schemas, layers) belong in their own module with their own tests
- Cross-crate models go in a `shared` crate's `lib.rs`
- Keep shared/library types as pure data types — don't add policy or business-logic methods to them. Policy logic belongs in the crate that owns the decision. Only inherent behaviour (formatting, parsing, construction) belongs on the type itself.
- Testing:
  - Always use `cargo nextest run` to run tests, never `cargo test`
  - Core functionality gets inline unit tests
  - Multi-part integration gets integ tests (use captured real data)
  - Prefer high-level invariant-based tests over bespoke examples; use [proptest](https://docs.rs/proptest/latest/proptest/) for property-based tests
  - Integration tests must use real application types (e.g. `App`, `Project` impls), not test-only duplicates
  - Integration tests should exercise the public HTTP interface for both setup and assertions — avoid calling store/service methods directly as back-doors when an HTTP endpoint exists for the same operation
- Binary layout:
  - All binaries must be named files in `src/bin/` (e.g. `src/bin/finder.rs`), never `src/main.rs` or subdirectories like `src/bin/finder/main.rs`
  - Modules used by binaries live under `src/` and are exposed through `lib.rs`, not placed alongside binaries in `src/bin/`
- Use `Option<T>` (with `None`) to represent "not set" / "disabled" — never use sentinel values like `0`, `-1`, or empty strings to encode absence
- Keep feature enablement flags (e.g. `--use-redis`) separate from their configuration values (e.g. `--redis-url`). A feature's on/off switch should not be derived from whether its config happens to be present — these are independent concerns.
- CLI apps: all config via named CLI params (`--long-form VALUE`); no env vars except `RUST_LOG`
- Prefer functions over macros — only use `macro_rules!` when you genuinely need syntax or control flow that a function can't express
- **Avoid `continue`.** It makes control flow indirect — the conditions for skipping live separately from the body of the loop. Prefer:
  - `Iterator::filter_map` / `filter` / `?` inside a closure for skip-on-condition patterns
  - Extract the inner body into a helper function that returns `Option<T>` and use `filter_map` over it
  - `if let Some(x) = ... { ... }` for guarded execution
  - Restructure so the loop body has a single straight-line path
- Prefer simple, readable techniques over clever ones until profiling identifies a real hotspot.
  - When the borrow checker pushes back, reach for `.clone()` first. Only escalate to tricks like `std::mem::take`, manual index loops, `RefCell`, or restructuring fields once a benchmark or trace shows the clone matters.
  - Example: in a per-tick loop that needs `&mut self` access while iterating one field, `let xs = self.xs.clone(); for x in xs { self.do_something(&x); }` beats `for x in std::mem::take(&mut self.xs) { ... }`. Both work; the clone is one line, obvious to read, and ~free for small Vecs of small values. Save the cleverness for when it's earned.
- We should aim to keep `lib.rs` files below 300 lines (found via a command like `find . -name "lib.rs" | grep -v "target" | xargs wc -l`). Any `lib.rs` file going above this limit should trigger us to apply other rules, for example related to extracting modules, that will allow us to split into into logical chunks.