# Bobby — Rust design-patterns review

## Recommendations (smallest → largest change)

> **Status (2026-06-22):** the `skeet-store` pass closed exactly one rust-patterns
> item — **`ParseZoneError`** (part of #1). Everything else lives in *other* crates
> and is scheduled under the per-crate passes in `docs/next-slices.md`, tagged below
> as **[firehose slice]** (*robust … firehose consumption + `skeet-prune` review/re-org*,
> Group 0 — `skeet-prune` + firehose-adjacent `bluesky`/prune config) or
> **[remaining-crates slice]** (*1.0 refactor … focussed on remaining crates*).

1. **Add `ParseRejectionError`** (§3) — one small enum replacing `type Err = String` in `Rejection`/`RejectionCategory` `FromStr`. (`ParseZoneError` ✅ done in the store pass.) → **[remaining-crates slice]**, Shared/support libs (`shared`).
2. **Validate `ModelProvider` and `Purpose` constructors** (§4). → **[remaining-crates slice]** — `ModelProvider` (`shared/refine_model.rs`) under the ML-libs + `skeet-refine` pass; `Purpose` (`eval`) under Shared/support libs.
3. **Close `&str` gaps at four call sites** (§2). → `bsky_cdn_thumbnail_url` + `blocked_labels` (`bluesky`) under **[firehose slice]**, Group 0; `SkeetId::for_post` (`shared`) + `FeedConfig` accessors (`skeet-feed`) under **[remaining-crates slice]** (Shared/support + Web services).
4. **Replace `Box<dyn Error>` in `PruneConfig::from_file` and `BlocklistConfig`** (§7) — both prune config in `shared`. → **[firehose slice]**, Group 0.
5. **Tighten `pub mod` to `mod` + `pub use`** in `skeet-appraise`, `skeet-feed`, `skeet-publish` (§6). → **[remaining-crates slice]** (Web services + Publishing). Low-priority; do when already touching them.
6. **Skip unless revisiting architecture:** TypeState for pipeline assembly (§5), zero-copy views (§8), combinator-style filter composition (§9) — all assessed as wrong fit or insufficient payoff.

A read of the full codebase (~17 crates, ~20k LOC) through the lens of
Chapters 9–13 of *Design Patterns and Best Practices in Rust* (Packt, 2026)
— architectural patterns, type-system patterns (NewType, parse-don't-validate,
TypeState, sealed traits), functional programming patterns, and Rust-core
idioms. This is **not** a checklist to action — it's observations grounded in
the book's advice, ordered roughly by impact, for you to weigh.

The book builds a "Samsa" pub/sub microservice across five chapters to
illustrate each pattern. Bobby's domain is different (ML pipeline over
Bluesky's firehose) but the structural advice transfers directly.

---

## TL;DR — the shape of it

Bobby already applies most of the book's patterns — often better than the
book's own examples. The NewType discipline is **exceptionally strong** (~30+
validated wrapper types), the data-flow architecture is textbook
unidirectional, mutability is well-contained, and error handling is
consistent `thiserror` throughout. The codebase reads like it was written by
someone who already internalised these principles.

The patterns most worth your attention:

- **A handful of primitive-obsession gaps** where validated NewTypes exist but
  aren't used at the call boundary (3–4 sites).
- **Inconsistent `FromStr` error types** — three types use `String` where
  every other type has a dedicated error.
- **Unvalidated NewType constructors** — two wrapper types accept any input,
  breaking "parse, don't validate."
- **No TypeState usage** — one moderate candidate exists (pipeline assembly)
  but the payoff is debatable.
- **Over-broad `pub mod`** in web-facing crates — low-priority but against
  the book's "modules become interfaces" principle.

---

## 1. What's working well (so the rest is calibrated)

### 1.1 NewType pattern — textbook, often exceeding the book

The book's Ch 10 NewType pattern: wrap primitives in distinct types with
validating constructors, dedicated error types, and zero-cost abstraction.
Bobby does this ~30 times with a consistent recipe:

1. `struct Foo(inner)` with validating `new()` → `Result<Self, InvalidFoo>`
2. `impl FromStr` delegating to `new()`
3. `impl Display` for canonical representation
4. `impl Serialize/Deserialize` via string round-trip
5. Custom `Eq`/`Ord` for floats (`f32::total_cmp` / `f64::total_cmp`)
6. Dedicated error type per NewType

Examples: `Score`, `NormalizedScore`, `Threshold`, `Percentage`, `ImageId`,
`SkeetId`, `BlueskyCid`, `Did`, `Nsid`, `RecordKey`, `ImageUrl`,
`ModelVersion`, `ModelProvider`, `ModelName`, `RefinePrompt`, `Label`,
`Limit`, `Order`, `DiscoveredAt`, `OriginalAt`, `AccountTag`, `ApiToken`,
`BucketName`, `Usd`, `Precision`, `Recall`, `F1`, `RocAuc`, `RunId`,
`Purpose`, `Appraiser`, `Band`, `Zone`, `Rejection`.

The book's Samsa example defines three NewTypes (`TopicId`, `ConsumerId`,
`MessageId`). Bobby has ten times that, all following the same disciplined
pattern. This is the strongest alignment with the book's advice across the
whole codebase.

### 1.2 Unidirectional data flow — Ch 9's core principle, natively

The book's #1 architectural principle: data flows in clear, predictable
directions; when bidirectional communication is needed, create a separate
downward channel.

Bobby's pipeline is a clean unidirectional flow:

```
Firehose → Meta filter → Image filter → Save → Refine → Publish → Feed
```

Stages communicate via typed `tokio::sync::mpsc` channels with enum messages
(`MetaResult`, `ImageResult`) at `skeet-prune/src/pipeline.rs:9,17`. Data
flows strictly downward. The only bidirectional interaction is
`skeet-appraise` ↔ `skeet-store` (the admin UI reads and writes appraisals),
which is clean and intentional — it's a separate concern, not upstream
feedback in the pipeline.

This matches the book's Samsa architecture (Producer → Broker → Consumer)
but with more stages and typed channel messages instead of topic strings.

### 1.3 Contained mutability — Ch 9's second principle

The book: confine mutability to specific, well-defined components; make all
state changes explicit and controlled.

Bobby's approach:
- **Interior mutability only where justified:** `RwLock` for version-gated
  score cache (`skeet-store/src/lib.rs:72`), `Mutex` for TTL-based existence
  caches (`bluesky/src/existence.rs:94-95`), `AtomicU64` for pipeline
  throughput counters (`skeet-prune/src/pipeline.rs:25`).
- **`&self` dominates public APIs** — shared references everywhere, mutable
  state behind synchronisation primitives.
- **`unwrap`/`expect` denied by workspace clippy lint** (`Cargo.toml:6-7`),
  each use carries `#[allow]` with justification.

The book's Samsa wraps state in `Arc<Mutex<>>` on the broker; Bobby does the
same but more granularly — each concern gets its own synchronisation
primitive with the appropriate choice (Mutex for short critical sections,
RwLock for read-heavy caches, atomics for counters).

### 1.4 Error handling — consistent `thiserror`, no `anyhow`

The book uses `thiserror` for its Samsa error type. Bobby goes further:
- Every crate has its own `thiserror` enum with `#[from]` on foreign-error
  variants and structured fields.
- Per-NewType validation errors (`InvalidScore`, `InvalidImageId`,
  `SkeetIdError`, `InvalidBlueskyCid`, etc.).
- No `anyhow` anywhere.
- `Box<dyn Error>` limited to binary `main()` functions (acceptable) plus
  two library methods (flagged below).

### 1.5 Functional patterns — Ch 11/12, used throughout

Iterator chains (`filter_map`, `map`, `collect`), `Option`/`Result`
combinators (`and_then`, `or_else`, `map_err`, `ok_or_else`), modern Rust
idioms (`is_none_or`, `then_some`), and builder patterns
(`StaticExistenceChecker::all_present().with_missing_skeets(…)`) appear
throughout.

### 1.6 Trait-based abstraction — Ch 9's "modules become interfaces"

`ExistenceChecker`, `FeedSource`, `PublishedImagesSource`,
`ImageUrlResolver` — all async traits injected as `Arc<dyn Trait>` with
real and test implementations. This matches the book's `StorageBackend`
trait pattern but avoids over-abstraction: there's no `trait Store` to
abstract the LanceDB backend (correctly — `docs/versioning.md` says a
backend change is "an entirely new top-level store, never an in-place
migration").

---

## 2. Remaining primitive obsession (Ch 10 — NewType)

Despite the strong overall discipline, a few APIs accept bare `&str` where
validated NewTypes exist in the same codebase:

| Location | Issue | Fix |
|---|---|---|
| `bluesky/src/image_url.rs:28` | `bsky_cdn_thumbnail_url(did: &str, cid: &str)` | Take `&Did`, `&BlueskyCid` |
| `shared/src/skeet_id.rs:34` | `SkeetId::for_post(did: &str, rkey: &str)` | Take `&Did`, `&RecordKey` |
| `skeet-feed/src/feed_config.rs:52-68` | `did()`, `feed_uri()`, `service_endpoint()` return raw `String` | Return domain types |
| `bluesky/src/post_thread.rs:51` | `blocked_labels` is `Vec<String>` | `Vec<Label>` |

The book's argument (Ch 10, "Identifying the problem"): "If you've worked
with similar systems, you've probably experienced the confusion this
creates. The NewType pattern solves this problem with compile-time type
checking." The codebase is *internally inconsistent* — most boundaries are
typed, but these four leak raw strings through.

**Impact:** low — these are narrow, low-traffic call sites. But they're easy
to close and completing the pattern prevents future callers from passing the
wrong `&str`.

---

## 3. Inconsistent parse errors (Ch 10 — Parse, don't validate)

The book's principle: "Once you have a `TopicId`, you *know* it is valid.
The type system carries the proof."

Bobby follows this for ~25 NewTypes, each with a dedicated error. Three
exceptions use `type Err = String`:

- `Zone::FromStr` — ✅ **done** (now returns `ParseZoneError`; closed in the store pass)
- `Rejection::FromStr` at `shared/src/rejection.rs:21`
- `RejectionCategory::FromStr` at `shared/src/rejection.rs:91`

Every other `FromStr` in the codebase returns a dedicated error
(`ParseBandError`, `ParseAppraiserError`, `InvalidScore`, etc.). These three
break the pattern, and downstream code that catches the parse failure gets an
opaque `String` instead of a matchable variant.

**Fix (remaining):** add `ParseRejectionError` (covering `Rejection` +
`RejectionCategory`), matching the recipe used everywhere else; `ParseZoneError` is
already done. Scheduled in `docs/next-slices.md` → *remaining-crates* slice,
Shared/support libs (`shared`).

---

## 4. Unvalidated NewType constructors (Ch 10 — Parse, don't validate)

The book: "The NewType pattern combined with early validation — once you have
`TopicId`, you *know* it is valid." Two Bobby NewTypes break this:

- **`ModelProvider(String)`** at `shared/src/refine_model.rs:10` — accepts
  any string with no validation. Only has an `openai()` factory, but the
  `new()`-equivalent path is open. A provider name that doesn't match any
  known provider silently propagates.
- **`Purpose`** at `eval/src/results.rs:62` — accepts empty strings. An
  empty purpose is meaningless but constructs without error.

The book would say: if the constructor can't fail, the type isn't doing its
job. Either validate (non-empty, known set) or document why any value is
acceptable.

**Impact:** low — both are internal types with few constructors in practice.
But they weaken the "if it compiles, it's valid" guarantee the rest of the
codebase builds on.

---

## 5. TypeState pattern (Ch 10) — one candidate, debatable payoff

The book's TypeState pattern: encode state machines in the type system so
invalid transitions are compile errors. Example: a connection that's
`Connection<Disconnected>` → `Connection<Connected>` → `Connection<Subscribed>`,
where calling `.subscribe()` on a `Disconnected` connection won't compile.

Bobby has **no TypeState usage**. One moderate candidate:

**Pipeline assembly in `skeet-prune`:** The pipeline is assembled
imperatively — channels are created, stages are spawned, handles are
collected. A `Pipeline<Unconfigured>` → `Pipeline<Connected>` →
`Pipeline<Running>` encoding could prevent calling `.run()` before channels
are wired. But the current code is ~50 lines of straightforward sequential
setup with no observed bugs — the TypeState ceremony would exceed the
safety payoff.

**Other patterns from Ch 10 that Bobby doesn't need:**
- **Sealed traits** — Bobby's traits are either internal or intentionally
  open for test doubles. No need to restrict implementors.
- **PhantomData-based schema typing** — the book's `TypedMessage<S>` pattern
  (messages generic over a schema type) doesn't map to Bobby's pipeline,
  where message types are concrete and flow through channels.

**Take:** the absence of TypeState is a reasonable design choice, not a gap.
The book itself notes it's "particularly valuable … for resources that must
follow strict lifecycle rules" — Bobby's resources don't have that shape.

---

## 6. Module visibility (Ch 9 — Modules become interfaces)

The book: "The `lib.rs` file acts as a gateway, explicitly choosing which
types to expose publicly while keeping implementation modules private."

Bobby does this well in `shared` and `skeet-store` (private `mod` with
selective `pub use` re-exports). But three web-facing crates expose nearly
everything:

- `skeet-appraise/src/lib.rs` — 12 `pub mod` declarations + 7 `pub use`
- `skeet-feed` — most modules `pub mod`
- `skeet-publish` — most modules `pub mod`

Since these are binary-adjacent crates consumed mainly by their own `main()`,
the practical risk is low. But the book's argument applies: "This creates a
stable public interface that can remain unchanged even when internal
implementation evolves." If any of these crates ever becomes a dependency of
another, the over-broad visibility creates accidental coupling.

**Impact:** low — these are leaf crates. Worth tightening if you're already
touching them, not worth a dedicated pass.

---

## 7. `Box<dyn Error>` in library code (Ch 9/12 — error design)

The book builds typed `SamsaError` enums and custom `Result` aliases. Bobby
does the same everywhere except two library methods:

- `PruneConfig::from_file` at `shared/src/lib.rs:119`
- `BlocklistConfig` methods at `shared/src/blocklist.rs:19,27`

These return `Box<dyn std::error::Error>`, which is the accepted pattern for
binary entry points but not for library functions — callers can't match on
the error or add context. The rest of the codebase uses typed errors
consistently; these two are outliers.

**Fix:** add variants to the relevant `thiserror` enum (likely `StoreError`
or a new `ConfigError`) wrapping the underlying IO/parse errors.

---

## 8. Zero-copy views (Ch 9 — Pack lightly)

The book's `MessageView<'a>` pattern: borrow instead of clone when
components only need to read data, using lifetime parameters to guarantee
safety.

Bobby doesn't use borrowing views — most types are `Clone` value types
passed by ownership or `&self` reference. The hot paths where views could
help:

- `scored_summaries()` in `skeet-publish` builds `Vec<(StoredImageSummary,
  Score, ModelVersion)>` — these are cloned for filtering and sorting.
- Pipeline messages (`MetaResult`, `ImageResult`) carry owned data through
  channels — ownership transfer is correct here (channels consume the value),
  so views don't apply.

**Take:** the clone-based approach is appropriate for Bobby's throughput.
Zero-copy views add lifetime complexity that's only justified at much higher
message rates. The book's own caveat: "When lifetimes become complex
(crossing thread boundaries, for example), cloning may be the simplest
solution." Bobby's async/channel architecture is exactly that case.

---

## 9. Functional patterns already in place (Ch 11/12)

Areas where Bobby already matches or exceeds the book's advice:

| Book pattern | Bobby usage |
|---|---|
| **Closure-based filtering** (Ch 11, `MessageFilter<F>`) | Builder-pattern filters in `StaticExistenceChecker` |
| **Iterator composition** (Ch 11, `filter_map` + `fold`) | Throughout `scores.rs`, `visibility.rs`, `publisher.rs` |
| **`Result`/`Option` combinators** (Ch 12) | `map_err`, `and_then`, `ok_or_else`, `is_none_or` chains everywhere |
| **Block expressions** (Ch 12, processing stages) | Used implicitly; Rust code naturally scopes bindings |
| **`From`/`Into` conversions** (Ch 10/12) | `From<Score> for f32/f64`, `From<Score> for Threshold`, etc. |
| **Builder pattern** (Ch 11) | `StaticExistenceChecker`, `StoreArgs` via `clap::Args` |

One pattern from the book not used but potentially useful:

**Combinator-style filter composition** (Ch 11, `filter.and(other_filter)`):
The book builds `MessageFilter` with `.and()` to combine predicates.
Bobby's visibility/publishing logic in `skeet-publish` composes filtering
inline via iterator chains, which is simpler and more idiomatic. The
combinator approach would only pay off if filters needed to be built
dynamically at runtime from user configuration — not the current case.

---

## 10. Patterns the book warns against — Bobby avoids them

The book's Ch 13 ("Leaning into Rust") and earlier anti-pattern chapters
flag several pitfalls. Bobby avoids all of them:

| Anti-pattern | Bobby's status |
|---|---|
| **`Rc<RefCell<T>>` everywhere** — fighting the borrow checker with shared mutable state | Not used anywhere |
| **Excessive cloning** — cloning to satisfy the borrow checker instead of designing ownership | Cloning is deliberate and appropriate (channel sends, Arc-wrapped shared state) |
| **`Arc<Mutex<>>` as default** — using smart pointers reflexively instead of ownership | Used only for genuinely concurrent shared state; justified at every site |
| **God objects** — one type owning everything | `SkeetStore` is the closest candidate (flagged in `skeet-store-review.md`); pipeline stages are well-separated |
| **Stringly-typed APIs** — bare `String` for domain concepts | Rare (§2 above); the norm is validated NewTypes |
| **`anyhow` in libraries** | Not used at all |
| **Fighting the borrow checker** with complex lifetime annotations | Lifetimes are minimal; most code uses owned types or `&self` |

---

## 11. If you only touch a few things

Ordered by value-per-effort:

1. **Close the `&str` gaps** (§2). Four sites where existing NewTypes aren't
   used at the boundary — `bsky_cdn_thumbnail_url`, `SkeetId::for_post`,
   `FeedConfig` accessors, `blocked_labels`. Small changes, each completing
   a pattern already established.

2. **Add `ParseZoneError` and `ParseRejectionError`** (§3). Two small error
   enums replacing `type Err = String`, matching the recipe used by every
   other `FromStr` in the codebase.

3. **Validate `ModelProvider` and `Purpose` constructors** (§4). Add
   non-empty checks (and optionally known-provider checks for
   `ModelProvider`) so the "if it compiles, it's valid" guarantee holds.

4. **Replace `Box<dyn Error>` in `PruneConfig::from_file` and
   `BlocklistConfig`** (§7). Typed errors matching the rest of the codebase.

5. **Tighten `pub mod` to `mod` + `pub use`** in `skeet-appraise`,
   `skeet-feed`, `skeet-publish` (§6). Low-priority; do when already
   touching these crates.

Design-level, skip unless revisiting architecture: TypeState for pipeline
assembly (§5, debatable payoff), zero-copy views (§8, wrong throughput
regime), combinator-style filter composition (§9, current approach is
simpler).

---

## 12. How this compares to the book's Samsa

| Dimension | Samsa (book) | Bobby |
|---|---|---|
| NewTypes | 3 (`TopicId`, `ConsumerId`, `MessageId`) | ~30+, with consistent validation recipe |
| Error handling | One `SamsaError` enum | Per-crate `thiserror` enums + per-type validation errors |
| Data flow | Producer → Broker → Consumer | 7-stage pipeline with typed channel messages |
| Mutability | `Arc<Mutex<>>` on the broker | Granular: `RwLock`/`Mutex`/`AtomicU64` per concern |
| Trait abstraction | `StorageBackend` trait with factory | `ExistenceChecker`, `FeedSource`, `ImageUrlResolver` — injected, testable |
| Module visibility | Private modules, selective `pub use` | Same in core crates; over-broad in leaf crates |
| TypeState | Full `Connection<State>` example | Not used — not needed given the domain |
| Functional patterns | Iterator chains, closure filters | Same + `is_none_or`, `then_some`, builder pattern |

Bobby is a more mature, production-grade codebase than Samsa. The book's
patterns are already internalised; the remaining gaps are minor consistency
issues, not missing concepts.

---

## Sources

- *Design Patterns and Best Practices in Rust*, Packt 2026, Chapters 9–13
  (Architectural Patterns, Type System Patterns, Functional Programming
  Patterns, Patterns from Rust's Core Features, Leaning into Rust)
- Bobby codebase at `slice-prod-staging-split` worktree, 17 crates
- `skeet-store-review.md` (prior review, referenced for overlap)
- Rust API Guidelines, `rust-unofficial/patterns` catalogue
