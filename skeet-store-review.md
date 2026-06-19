# `skeet-store` — structure & patterns review

A deep read of the crate (~6.3k LOC, 34 source files) compared against current
Rust-systems advice and how other Rust data-engineering codebases are shaped.
This is **not** a checklist to action — it's a set of observations grounded in
external advice, ordered roughly by impact, for you to weigh against your own
opinions.

Versions pinned while writing: `lancedb 0.27.2`, `lance 4.0.0`, `arrow 57`.
Findings cite `file:line` from the worktree at review time.

---

## TL;DR — the shape of it

`skeet-store` is really **three crates wearing one trench-coat**:

1. **A LanceDB-backed store** for images / scores / appraisals — the actual job.
2. **Store-level observability** — `r2_metrics` (object-store decorator),
   `store_metrics`, `query_plan` + slow-query logging. Store-adjacent, defensible.
3. **Unrelated trace tooling** — `tempo` (Grafana Tempo HTTP client) +
   `trace_analysis` (trace-tree summariser), ~850 LOC / 13% of the crate, with
   its own error type, touching nothing in the store. This belongs elsewhere.

The store itself is **well-built and unusually well-documented** (the versioning
design doc is genuinely excellent), with strong NewType discipline, a clean
`thiserror` enum, good proptest coverage, and a nice decorator over
`object_store`. The patterns most worth your attention:

- **String-interpolated SQL predicates everywhere** (`only_if(format!("… = '{id}'"))`,
  14 sites) where a typed/escaped path exists. The highest-leverage finding.
- **Two upsert idioms, one of them non-atomic** (`delete`-then-`add` vs
  `merge_insert`).
- **One `SkeetStore` god-type owns all 5 tables** behind `pub(crate)` fields; the
  file-splitting is good, the type-level cohesion is the open question. Full
  interface/trait/struct treatment in the **★ section** below.
- **Read paths materialise whole tables and compute in Rust** — mostly mitigated
  by caching, but paging is the real outlier.
- **`#[instrument]` on per-row hot helpers**.

---

## Recommendations — smallest → largest change

The full set, ordered by size of change. Everything through the **Medium** tier is
doable on your current pins (`lancedb 0.27.2` / `lance 4.0.0`) — no upgrade needed.
Section refs point into the detail below. (§9 reorders a subset by
value-per-effort; this list is the complete by-size view.)

**Trivial — minutes, delete/one-liner**
1. Delete the dead `StoreError::InvalidUri` variant (§4).
2. Drop `#[instrument]` from per-row helpers — `to_summary`, `extract`, `typed_column`, `encode_image_as_png` (§6).
3. Remove the vestigial `Option` on `store_wrapper` — `open()` always sets `Some` (§E).

**Small — one signature + a few call sites**
4. `list_scores_for_ids(&[&str])` → `&[ImageId]` — types away the one real injection hole (§3.1/§E).
5. `get_by_ids` / `get_originals_by_ids` → `HashMap<ImageId, _>` — your own rule; `loader.rs` already hand-rolls the map, so this deletes caller code (§E).
6. Typed error variants (`#[from]`/`#[source]`) instead of `ValidationFailed(String)` / `InvalidZone(String)` (§4).
7. Consolidate upserts on `merge_insert` — make test-only `upsert_score` a one-row wrapper over `batch_upsert_scores`; switch production `set_*_band` off `delete`-then-`add` (§3.2).

**Medium — a module / cross-cutting pattern (still no upgrade)**
8. Typed filters via `only_if_expr` (verified injection-proof on native tables), plus an `escape_literal` stopgap for any string predicates kept (§3.1).
9. Name the boundary tuples — `ScoredSummary { summary, score, model_version }` and a score+version pair (§E).
10. `TableName` enum to replace stringly table identity across registry/errors/metrics (§E).
11. Push paging + counts/distinct down to the engine via `as_native()` → lance `Scanner` `order_by`/`limit`/`aggregate` (§3.3/§3.4).
12. DRY/declarative refactors: `FromRecordBatch` decode trait (§C), declarative `TableSpec` to collapse `open()` (§7), generic `AppraisalTable<K>` for the duplicated appraisal CRUD (§D).

**Large — architectural (highest payoff, most churn)**
13. Make the table fields private; force cross-table access through methods — the lever for everything below (§A).
14. Read/write capability split (`trait ImageReader`/`ScoreReader` or a `ReadStore`) so feed/publish *can't* write — encodes your prod/staging safety rule (§B.3).
15. Sub-structs by use-case/read-model (or per-table) — reshape `SkeetStore` from a 35-method god-type into a façade (§B).
16. Extract `tempo` + `trace_analysis` into their own crate — ~850 LOC, big move but low risk (§2.1).

**Strategic — separate projects / judgement calls**
17. Blob v2 for the PNGs — lance-dataset-level, needs an `images_v7` bump; compaction/storage win, not read-latency (§11.1).
18. lancedb `0.30` upgrade — buys `order_by` ergonomics + Lance Namespace + unenforced PKs, but costs **arrow 57→58 (workspace-wide) + lance 4→7**; do it *after* the no-upgrade fixes (§11.2).
19. Adopt DataFusion-direct incrementally — register datasets as `LanceTableProvider` for complex reads; additive, data stays in Lance; subsumes #11 (§10.2).
20. Iceberg — **not recommended**: format migration + workload mismatch; if the draw is the catalog, Lance Namespace (#18) gets there without leaving Lance (§10.3).

*Suggested sequence (not pure size order):* do **1–7** as a quick cleanup pass,
then **8 + 11** (the injection + pushdown wins that need no upgrade), then decide
whether **13–15** are worth the reshape before anything strategic.

---

## 1. What's working well (so the rest is calibrated)

- **NewType discipline** is strong and consistent: `DiscoveredAt`, `OriginalAt`,
  `SkeetId`, `ImageId`, `Score`, `ModelVersion`, `Version` — no bare `String`/`f32`
  leaking through the public API. This is exactly the rust-by-example NewType
  idiom your own `.claude/rules/rust.md` calls for.
- **`thiserror` enum** (`error.rs`) is idiomatic: `#[from]` on the foreign-error
  variants, structured fields (`ColumnTypeMismatch { column }`,
  `LimitExceeded { requested, maximum }`). This is the recommended shape.
- **`VersionedCache`** (`versioned_cache.rs`) is a small, single-responsibility,
  well-documented, well-tested abstraction that serves both value-caches and
  skip-gates. Caller-owns-synchronisation is the right call.
- **`R2MetricsWrapper`** (`r2_metrics.rs`) is a textbook **decorator** over
  `object_store::ObjectStore` — the idiomatic way to instrument Lance's I/O
  without forking it. The `MetricsRecorder` builder + R2 A/B billing-class
  labelling is thoughtful, and it's the best-tested file in the crate.
- **Observability of queries**: parsing DataFusion's `explain_plan` text into
  `QueryPlan` and emitting flat fields + a slow-query threshold
  (`lancedb_utils.rs`) is more than most projects bother with, and the
  `unknown_keys` "tell me when the format grows" mechanism is a nice touch.
- **Test coverage**: proptest for time-window logic (`types.rs`) and the plan
  parser; colocated unit tests; feature-gated `test_utils` (`test-helpers`) so
  helpers are reusable across crates without leaking into release builds.
- **`docs/versioning.md`** is exemplary — the prod/staging coexistence rules,
  the covariance/contravariance framing, and the read-time known-versions filter
  are the kind of thing most teams discover the hard way.

---

## ★ Interface, trait & struct design (your priority)

This section goes deeper on the design surface — what the types/traits/modules
*say to a caller* — since that's what you flagged. It overlaps a little with
§2/§5 below but is the part to read first.

### A. The mechanism behind the god-type: `pub(crate)` tables

`SkeetStore`'s five tables, the cache, and the wrapper are all `pub(crate)`
(`lib.rs:64-74`). That single decision is *why* the type can sprawl: `scores.rs`
reads `self.images_table` directly (`scores.rs:221`, `253`), `summary.rs` reads
both tables, `optimise.rs` walks `self.tables`. There is no encapsulation seam —
every module in the crate has full access to every table. Whatever you decide
about sub-structs, the lever is here: **the day those fields stop being
`pub(crate)`, cross-table access is forced through named methods**, and the shape
of the crate changes on its own. Right now the *implementation* leaks across the
whole crate even though the *public* API looks tidy.

### B. Three ways to carve the interface

The current carve is **none** — one type, ~35 methods, grouped into files by
table. Three principled alternatives:

1. **By table (classic repository).** `Images`, `Scores`, `Appraisals`,
   each owning one `lancedb::Table`; `SkeetStore` a façade with
   `store.images()`, `store.scores()`. *Cost specific to this codebase:* the
   central read paths are **cross-table joins** (`fetch_summaries_for_scores`,
   `find_recent_image_ids`, `list_scored_summaries_*` all read scores *and*
   images). Those can't live on `Scores` (it can't see images) — so they'd move
   to the façade. That's arguably *more honest*: they were never "scores
   operations," they're feed read-model operations mislabelled by which file they
   landed in.

2. **By use-case / read-model.** Group by what callers do, not by table:
   an *ingest* concern (`add`, `upsert_score` — written by prune/refine), a *feed
   read-model* (the scores⋈images joins — read by publish/feed), an *appraisal*
   concern (admin UI), a *maintenance* concern (`optimise`/`prune`/`health`).
   The call-site histogram already clusters this way (`add` ×78, the feed reads,
   the appraisal reads, the cron maintenance). This matches the *consumers* best.

3. **By read/write capability** — the one I'd weigh most, because it
   *encodes a safety property you already care about.* Your `docs/versioning.md`
   leans hard on "readers are covariant; only writers are dangerous; never run a
   staging **writer** against the shared store," and you enforce it today with a
   runtime CLI flag (`--allow-shared-store-write`). The type system can carry
   part of that: a read-only interface (`trait ImageReader`/`ScoreReader`, or just
   a `ReadStore` newtype that only exposes the read methods) that `skeet-feed` and
   `skeet-publish` depend on. They then *cannot* call `add`/`upsert`/`delete` —
   the "covariant reader" of your doc becomes a compile-time fact, not a
   convention. The concrete `SkeetStore` implements both halves; writers take the
   full type.

My leaning: **B or C over A.** A (per-table) is the textbook answer but fights the
cross-table joins that are the heart of this store; B/C follow the grain of how
the crate is actually used and (for C) make a documented safety rule structural.

### C. Where traits earn their keep here — and where they don't

The crate currently defines **no domain traits** (only impls of *external* ones:
`WrappingObjectStore`, `ObjectStore`, `Display`, `From`, `clap::Args`). That's not
a deficiency by itself — the repository literature pushes traits mainly for
mocking, and you've sidestepped that need by testing against a **real** local
store over a tempdir (`test_utils::open_temp_store`), which is *better* than mocks.
So resist trait-for-mockability. Traits that would pull their weight:

- **Reader/writer capability traits** (§B.3) — value is *capability narrowing +
  safety*, not abstraction. The strongest candidate.
- **`FromRecordBatch` (or `rows(&batch) -> impl Iterator`)** — a small *internal*
  trait to kill the 6 near-identical "extract typed columns, loop `0..num_rows`,
  build struct" blocks (`stored.rs`, `scores.rs:321-333`, `appraisals.rs:176-188`,
  `summary.rs:86-92`, `lib.rs:312-322`). Co-locates the column-name list with the
  struct it builds. Pure internal-implementation win, low risk.
- **A `TableSpec` trait / declarative spec** — `{ name, schema(), indexed_columns,
  compaction_options }` per table, iterated by `open()` and by
  `maintenance_tables()`. Collapses the 5× create-if-missing + 4–6× index-if-
  missing boilerplate (§7) and makes "add a table" one impl. This is the
  data-driven version of the `tables` registry the code already gestures at.

And explicitly where a trait would be **ceremony, not design**:

- **A `trait Store` to abstract the backend** (swap LanceDB for X). Your
  versioning doc says a backend change is "an entirely new top-level store, never
  an in-place migration" — i.e. the backend *by design* never varies behind a
  stable interface. Abstracting over a thing that never varies is the
  over-application the [repository-in-Rust thread](https://users.rust-lang.org/t/is-the-repository-pattern-a-viable-pattern-in-rust/25030)
  warns about. Skip it.

### D. A generic struct that removes a whole duplicated interface

`appraisals.rs` implements the *same* CRUD twice — `set_skeet_band` /
`set_image_band`, `get_skeet_band` / `get_image_band`, `clear_*`, `list_all_*` —
differing only in key type (`SkeetId` vs `ImageId`), table, and id-column name.
The schema (`appraisal_schema(id_column)`) and the parse (`parse_keyed_appraisals`)
are already shared; the *interface* is the only thing duplicated. A generic
**`AppraisalTable<K>`** (`struct AppraisalTable<K> { table: Table, id_column:
&'static str }`) with `set/get/clear/list_all` written once, instantiated as
`AppraisalTable<SkeetId>` and `AppraisalTable<ImageId>`, halves that surface. This
is a clean example of "generic struct over a key type" beating two hand-written
copies — and it's the natural first sub-store if you go the §B route.

### E. Concrete signature / type smells

- **Positional `Vec` returns that should be identity-keyed maps — violates your
  own rule.** `get_by_ids(&[ImageId]) -> Vec<StoredImage>` (`lib.rs:188`) and
  `get_originals_by_ids -> Vec<StoredOriginal>` (`lib.rs:201`) return Vecs whose
  order is engine-defined (`IN (…)`), with no positional correspondence to the
  input ids. Your `rust.md` is explicit: *"Identity-keyed maps over
  positionally-aligned Vecs … return `HashMap<Id, Output>`."* And the evidence is
  right there — `skeet-refine/src/loader.rs:57-60` takes the Vec and **immediately
  rebuilds a `HashMap<ImageId, DynamicImage>` by hand.** The caller is doing the
  method's job. `list_scores_for_ids` already returns `HashMap<ImageId, …>`
  (`scores.rs:402`) — so the codebase is *internally inconsistent* with its own
  rule. Make the two `*_by_ids` methods return maps; `loader.rs` then deletes its
  workaround and `eval.rs`'s `not_found` count stops depending on
  "missing ids just don't appear."

- **Name the tuples at the boundary.** `(Score, ModelVersion)` recurs ~8× as a
  return (`get_score`, the `ScoresMap` value, `list_scores_for_ids`), and the feed
  methods return `Vec<(StoredImageSummary, Score, ModelVersion)>`
  (`scores.rs:153,187`). Positional tuples at a public boundary make call sites
  read `x.1`/`x.2` and can't grow a field without breaking everyone. A
  `ScoredSummary { summary, score, model_version }` (and a `Score`+`ModelVersion`
  pair type if it travels together) is more readable and extensible — the same
  instinct as the NewType rule, one level up at the aggregate.

- **Table identity is stringly-typed across the whole crate.** The 5 table names
  are `const &str` (`schema.rs:5-9`), then flow as `&'static str` in the `tables`
  registry, as `String` in `CannotGetFragmentCount { table: String }`
  (`error.rs:41`) and `Version { name: String }` (`version.rs:10`), and as
  `table.to_string()` metric labels (`store_metrics.rs:25`). It's a **closed set
  of 5** masquerading as open strings. A `TableName` enum (with `as_str()` /
  `Display`) would make the set explicit, make "iterate all tables" exhaustive
  (the compiler reminds you when you add one), and remove the `&'static str` vs
  `String` mixing.

- **`store_wrapper: Option<Arc<dyn WrappingObjectStore>>` is vestigial.** `open()`
  always sets it to `Some(...)` (`open.rs:33,193`), so the `None` arm of
  `write_options()` (`lib.rs:108`) is unreachable in practice. It models a
  "no-metrics" state the constructor never produces. Either drop the `Option`
  (always wrap) or add a real no-metrics constructor that justifies it — right now
  it's an `Option` that's always `Some`, which is its own small anti-pattern.

- **`StoredImage` vs `StoredOriginal` — defensible, with a named cost.** These
  differ only by the presence of `annotated_image`. Modelling "optional field" as
  *two types* is actually a nice "make illegal states unrepresentable" move:
  callers that need the annotation get it guaranteed, no `Option` to `unwrap`.
  The cost is combinatorial — a third projection (say, summary + annotated only)
  means a third type and a third `batches_to_*`. If that day comes, a generic
  `Stored<P>` parameterised by projection, or composition
  (`summary` + `Option<Images>`), scales better. For two projections, the explicit
  pair is the right call; just know the boundary. The `summary`-as-a-field
  composition (`StoredImage { summary, image, annotated }`) is good either way.

- **`shared` vs `skeet-store` placement, and the re-export façade.** `lib.rs:33`
  re-exports `Appraiser, Band, ModelVersion, Score` from `shared`, so callers
  write `skeet_store::Score`. That's a deliberate façade (consumers depend on the
  store, not on `shared` directly) — reasonable, but it means every new shared type
  used at the boundary needs a re-export edit, and it blurs "where does this type
  live." Meanwhile `DiscoveredAt`/`OriginalAt` live *in* the store though the feed
  consumes them via summaries — arguably they're as "shared" as `Score`. Worth a
  deliberate rule: *domain value types in `shared`, storage-shaped types in the
  store*, and apply it consistently. Tied to your existing rule "cross-crate models
  go in `shared`; keep shared types pure data" — these timestamp newtypes are pure
  data and cross-crate, so they may be on the wrong side of the line.

---

## 2. Module boundaries & crate shape

### 2.1 `tempo` + `trace_analysis` don't belong here
*Evidence:* `tempo.rs:1-343` is a `reqwest`-based Grafana Tempo client with its
own `TempoError`; `trace_analysis.rs:1-509` summarises trace trees. Neither
references `SkeetStore`. They're consumed by `bin/trace-summary.rs`.

*External view:* the Rust API guidelines and the
[rust-unofficial/patterns](https://github.com/rust-unofficial/patterns) catalogue
both push "a crate/module is one cohesive responsibility." Your own rule —
*"different kinds of things … belong in their own module with their own tests"* —
is the same idea one level up.

*Take:* extract a `trace-tools` crate (or fold into an existing observability
crate). It would shed ~13% of the LOC, drop `reqwest`/`serde_json` from the
store's dependency surface, and stop "open the store crate to fix the trace
summariser" churn. `query_plan` is the one shared type; it can live in the new
crate (live query logging imports it) or in `shared`.

### 2.2 One `SkeetStore` type owns five tables
*Evidence:* `SkeetStore` (`lib.rs:63-75`) holds `images_table`, `scores_table`,
`validate_table`, `skeet_appraisal_table`, `image_appraisal_table` **and** a
`tables: Vec<(&'static str, Table)>` registry. ~35 public async methods are hung
off it across `lib.rs`, `scores.rs`, `appraisals.rs`, `paging.rs`, `summary.rs`,
`optimise.rs`, `version.rs`.

*External view:* splitting one type's `impl` across files is **idiomatic and
encouraged** ([users.rust-lang.org thread](https://users.rust-lang.org/t/code-structure-for-big-impl-s-distributed-over-several-files/7785));
that part is fine. The repository-pattern discussion
([is the repository pattern viable in Rust?](https://users.rust-lang.org/t/is-the-repository-pattern-a-viable-pattern-in-rust/25030),
[Leapcell on service/data layers](https://leapcell.io/blog/building-robust-business-logic-with-rust-web-service-layers))
is about the *type-level* boundary: one object that can touch every table from
any method is the "god object" the pattern exists to avoid.

*Take:* you don't need trait-based DI or a mock-everything abstraction (that's
the over-application the forum thread warns about). But consider per-table
sub-structs — `Images`, `Scores`, `Appraisals` — each owning its one
`lancedb::Table`, with `SkeetStore` as a thin façade exposing `store.scores()`,
`store.images()`. Today `scores.rs` reaching into `self.images_table`
(e.g. `find_recent_image_ids`, `fetch_summaries_for_scores`) is a cross-table
join living on the scores module; sub-structs make those cross-table reads
explicit (a method on the façade that calls both) rather than ambient.

### 2.3 The table registry duplicates the named fields
*Evidence:* every table is stored twice — as a named field and as an entry in
`tables` (`lib.rs:64-72`, populated `open.rs:178-194`). The doc comment is honest
about why (single iteration source), but it's two sources of truth kept in sync
by hand.

*Take:* if you keep the flat struct, derive iteration from the fields (a small
`fn tables(&self) -> [...]`). If you move to sub-structs (2.2), each owns its
table and the registry falls out naturally. `lancedb::Table` is `Arc`-cheap to
clone, so neither costs anything at runtime — this is purely about not hand-syncing.

---

## 3. The read / write path

### 3.1 String-interpolated predicates — the headline
*Evidence:* 14 sites build filters by interpolation —
`only_if(format!("image_id = '{image_id}'"))` (`lib.rs:181,231,241`),
`delete(&format!("skeet_id = '{skeet_id}'"))` (`appraisals.rs:31,63,99,131`),
`image_id IN ({list})` (`lib.rs:362-368`, `scores.rs:357-362,407-412`),
timestamp casts (`scores.rs:217-219`, `paging.rs:46-48`).

*External view:* `lancedb 0.27.2` offers **`only_if_expr(filter: datafusion_expr::Expr)`**
(`query.rs:424`) alongside the string `only_if` — backed by
`QueryFilter::{Sql, Datafusion, Substrait}` (`query.rs:698`). The
[filtering docs](https://docs.lancedb.com/search/filtering) only document
**column-name** escaping (backticks); there is **no value binding / parameterised
query** and no injection guidance for the string path. So with strings, escaping
values is entirely the caller's responsibility.

*Why it mostly works today, and where it doesn't:* most interpolated values are
NewType-validated (`ImageId`, `SkeetId`) so a stray `'` is unlikely — but that's
an *invariant you're trusting implicitly*, not enforcing at the query boundary.
The clear hole is **`list_scores_for_ids(image_ids: &[&str])`** (`scores.rs:399-412`):
raw `&str`, no validation, interpolated straight into `IN (…)`. A `'` there
breaks the query (and is a textbook injection shape).

*Take:* this is the highest-leverage change. Options, cheapest first:
  1. Take `&[ImageId]` not `&[&str]` in `list_scores_for_ids` (push validation to
     the type system — matches your "no bare Strings" rule).
  2. A single `escape_literal` helper (double the `'`) used by every `format!`
     predicate, so the invariant is enforced in one place rather than assumed.
  3. Move the equality/`IN`/range predicates to `only_if_expr` with
     `col("image_id").eq(lit(id.to_string()))` / `is_in(...)`. Type-safe, no
     escaping, reads like the domain.

*Verified (was a caveat): `only_if_expr` is injection-proof on your tables, and
needs no upgrade.* Reading `lancedb 0.27.2` source: the native execution path
applies a `QueryFilter::Datafusion(expr)` via **`scanner.filter_expr(expr.clone())`**
(`table/query.rs:220`) — the structured `Expr` goes straight into the Lance/DataFusion
scanner; **no SQL string is built**, so there is nothing to escape and no injection
surface. (The string round-trip via `expr_to_sql_string` exists only on the
*server-side / namespace* path, `table/query.rs:494`, and even that uses
datafusion's `unparser::Unparser` — `expr/sql.rs:31` — which quotes literals
correctly; the custom dialect only changes *identifier* quoting to backticks.) So
option 3 is available **today on 0.27.2** and is the most robust of the three.

### 3.2 Two upsert idioms — one non-atomic
*Evidence:* `batch_upsert_scores` uses `merge_insert(&["image_id"])` with
`when_matched_update_all` / `when_not_matched_insert_all` (`scores.rs:84-87`) —
the correct atomic upsert. But `upsert_score` (`scores.rs:29-49`),
`set_skeet_band` (`appraisals.rs:30-57`) and `set_image_band`
(`appraisals.rs:98-125`) all do **`delete(...)` then `add(...)`** as two separate
operations.

*External view:* Lance's `merge_insert` is "a delete and insert in a single
transaction"; Lance uses MVCC with atomic commits
([Lance read/write](https://lance.org/guide/read_and_write/),
[transactions](https://lance.org/format/table/transaction/)). `delete`-then-`add`
is two commits — a reader (or a crash) between them sees the row **absent**, and
two concurrent writers can interleave.

*Follow-up research sharpened this into two distinct findings:*

- **`upsert_score` is test-only.** Every caller is in `mod tests` or `/tests/`
  (`skeet-refine/src/polling.rs:108` is inside `#[tokio::test]` at line 99; the
  rest are `skeet-publish`/`skeet-appraise` test files). **Production scoring goes
  exclusively through the atomic `batch_upsert_scores`** (`live_refine.rs:134`).
  So the non-atomic single-row path never runs in production — but that's *worse*
  than it sounds two ways: (1) it's effectively dead production code (your "remove
  dead code" rule), and (2) the tests that use it exercise a **different write
  path than production**, so they don't validate the real `merge_insert` behaviour
  — which your rule "avoid back-doors when the real path exists" warns against.
  *Best fix:* make `upsert_score` a one-row wrapper over `batch_upsert_scores`
  (one implementation, atomic, and the tests now hit the production path), or
  delete it and have tests call the batch method.
- **`set_skeet_band` / `set_image_band` ARE production** — the admin appraisal UI
  (`skeet-appraise/src/admin.rs:341,348`). Here the non-atomic `delete`-then-`add`
  is live: a concurrent `get_skeet_band` between the two commits sees the row
  **absent**. Concurrency is low (human-driven, one appraiser at a time), so the
  practical risk is small, but the visibility gap is real and trivially removed by
  switching to `merge_insert` (Lance: "a delete and insert in a single
  transaction" — [PK upsert discussion](https://github.com/lance-format/lance/discussions/3842)).

### 3.3 Read paths materialise whole tables and compute in Rust
*Evidence:* `cached_scores` full-scans the scores table into a `HashMap`
(`scores.rs:294-345`); `unique_skeet_ids` scans + dedups in a `HashSet`
(`lib.rs:305-325`); `count_scored_images` / `count_scores_by_model_version` scan
+ count in Rust (`scores.rs:450-498`); `list_all_image_ids_by_most_recent` scans
then `sort_by` in memory (`lib.rs:346-357`).

*External view:* the columnar-engine principle is "push selection, projection,
ordering, aggregation **down to the engine**; stay in Arrow as long as possible;
materialise to row structs last." It's why DataFusion exists under Lance.

*Take — but be fair to the constraints:*
  - The **scores** work is well-justified: it's version-gated cached
    (`VersionedCache`), the scores table is small relative to images, and the
    known-versions filter is awkward to express as pushdown. Leave it.
  - `count_rows(None)` already pushes count down (`lib.rs:248`, `summary.rs:63`),
    so the model is understood — it's just applied unevenly.
  - The genuine outlier is **paging** (next item).

### 3.4 Paging is O(rows-before-cursor), not O(limit)
*Evidence:* `list_summaries_page` (`paging.rs:31-63`) applies the
`discovered_at < cursor` filter (good, pushed down) but then **collects every
matching row**, `sort_by` in memory, and only *then* truncates to `limit`. No
`.limit()` reaches the engine.

*Subtlety (this is why it's not just naïve):* the high-level `lancedb 0.27.2`
`Query` builder has `limit`/`offset` but **no `order_by`** (confirmed:
`query.rs` has `limit:382`, `offset:388`, `only_if:402`, `only_if_expr:424`, but
no ordering method). So you can't just add `.limit(limit+1)` — without ordering,
`limit` returns an arbitrary N, and your in-memory sort would then only see those
arbitrary N. The sort-in-Rust is a **workaround for a missing builder method**,
not carelessness.

*Take:* ordered top-N pushdown *is* reachable — the lower-level `lance 4.0.0`
`Scanner` has both `order_by(Vec<ColumnOrdering>)` (`scanner.rs:1598`, with
`ColumnOrdering::desc_nulls_last`) and `limit(limit, offset)` (`scanner.rs:1344`),
and you already drop to `table.as_native()` elsewhere (`lib.rs:93`). So a paging
query could scan the native dataset with `order_by(discovered_at desc)` +
`limit(limit+1)` and let the `discovered_at` scalar index do the work. More code,
and it bypasses some lancedb conveniences — so worth it **only once the images
table is large enough that "fetch everything older than the cursor each page"
hurts**. Flag it; don't necessarily fix it now.

### 3.5 Strong consistency on every read
*Evidence:* `read_consistency_interval(Duration::ZERO)` (`open.rs:28`) — every
operation re-checks the manifest. `optimise.rs:63-75` already documents the
downstream cost (manifests accumulate; every Strong read pays a growing R2 LIST)
and prunes hourly to bound it.

*Take:* this is a deliberate, documented correctness choice, not a bug. The only
thing worth revisiting: does *every* caller need ZERO, or could read-mostly paths
(feed serving) tolerate a few seconds of staleness for fewer R2 LISTs? That's a
per-CLI tuning question, not a store-internal one — possibly surface the interval
through `StoreArgs`.

---

## 4. Error handling

*Good:* the enum shape is idiomatic (see §1).

*Two snags, both flagged by current `thiserror` advice
([lpalmieri deep-dive](https://www.lpalmieri.com/posts/error-handling-rust/),
[thiserror #52 on catch-alls](https://github.com/dtolnay/thiserror/issues/52)):*

- **Stringly-typed swallowing of typed errors.** `Score::new(...)` failures are
  mapped to `ValidationFailed(e.to_string())` in three places
  (`scores.rs:110,328,431`), and `Zone` parse failures to
  `InvalidZone(value.to_string())` discarding the parse error
  (`stored.rs:79-80`). Both throw away a typed source. Prefer
  `#[from] shared::InvalidScore` / a `#[source]`-carrying variant so the chain
  survives — you already do this for `InvalidBand`, `InvalidAppraiser`,
  `InvalidImageId`, so it's an inconsistency more than a gap.
- **Dead variant.** `StoreError::InvalidUri(String)` (`error.rs:5`) is never
  constructed (`grep` confirms a single hit, the definition). Your own rule says
  "always remove dead code — git history is the archive." Drop it.

---

## 5. Type design & API surface

- **`archetype` vs `zone` naming mismatch.** The Arrow column is `archetype`
  (`schema.rs:66`) but the Rust field, the value written, and the parse target
  are all `zone`/`Zone` (`lib.rs:142`, `stored.rs:76-80`). A reader of the schema
  can't tell they're the same concept. If it's a legacy name, a one-line comment
  on the schema field would save the next person a `grep`; if not, an
  `images_v7` rename.
- **The batch→struct extraction pattern repeats.** `SummaryColumns` (`stored.rs`)
  nicely abstracts one table's row decode, but `scores.rs`, `appraisals.rs`,
  `summary.rs`, and `lib.rs` each re-hand-roll "`typed_column` × N, loop
  `0..num_rows`, `.value(i)`, build struct." A tiny `FromRecordBatch`-style trait
  (or a `rows(batch) -> impl Iterator`) would DRY ~6 near-identical loops and put
  the column-name list next to the struct it builds.
- **API width.** ~35 public async methods on one type is a lot of surface for
  callers to reason about (and `store.add(` alone has 78 call sites). Sub-structs
  (§2.2) would group it; even without that, grouping the `pub use` re-exports in
  `lib.rs` by concern would help orientation.

---

## 6. Observability & instrumentation

*Evidence:* `#[instrument]` is applied to **per-row / per-cell helpers**:
`SummaryColumns::to_summary` (called once *per row*, `stored.rs:74`),
`SummaryColumns::extract` (per batch, `stored.rs:61`), `typed_column` (per column
per batch, `arrow_utils.rs:10`), `encode_image_as_png` (per image,
`arrow_utils.rs:23`).

*External view:* the consistent guidance
([tracing instrument tutorial](https://gist.github.com/oliverdaff/d1d5e5bc1baba087b768b89ff82dc3ec),
[O'Reilly on tracing overhead & sampling](https://www.oreilly.com/library/view/distributed-tracing-in/9781492056621/ch06.html))
is "span significant operations, not every function call"; per-row spans are the
canonical over-instrumentation case — they add measurable overhead and bury the
useful spans in noise.

*Take:* instrument at the operation boundary (the public store methods, which you
already do) and drop `#[instrument]` from the row/cell/column helpers. If you want
visibility into decode cost, a single span around `batches_to_*` with a
`rows = N` field beats N spans.

---

## 7. `open()` boilerplate

*Evidence:* `SkeetStore::open` (`open.rs:19-195`, ~175 lines) repeats
"create-table-if-missing" 5× (`open.rs:46-76`) and "create-index-if-missing"
4–6× with the same `list_indices().any(columns == …)` shape
(`open.rs:83-155`).

*Take:* this is the "boilerplate that wants to be data" smell. A declarative
spec — `[(name, schema_fn, &[indexed_columns])]` — iterated once would collapse
all of it, make "add a table" a one-line edit (which the `tables` registry
comment already aspires to), and remove the risk of a new table silently missing
its index. This also dovetails with the sub-struct direction (§2.2): each
sub-store declares its own spec.

---

## 8. How this compares to other Rust data-engineering codebases

- **DataFusion / Lance themselves** layer hard: a low-level `Dataset`/`Scanner`
  (lance) under a high-level `Connection`/`Table` (lancedb), with `TableProvider`
  as the trait seam. `skeet-store` wraps the *high-level* `lancedb::Table`
  directly in one app type — fine for an app, but it means when the high-level
  API lacks something (ordering, §3.4) you have no seam to drop through except
  ad-hoc `as_native()`. A per-table sub-struct is where a `scan_ordered()` escape
  hatch would naturally live.
- **GreptimeDB / Databend** keep **schema/catalog**, **storage I/O**, and
  **query/exec** in separate crates or clearly separate modules
  ([greptime `table` crate](https://greptimedb.rs/table/index.html),
  [lib.rs database-implementations](https://lib.rs/database-implementations)).
  `skeet-store` mostly does this *within* the crate (`schema.rs` is isolated;
  `r2_metrics` is isolated) — the exception is the trace tooling (§2.1), which is
  a whole separate concern living inside.
- **The "stay columnar" instinct.** Mature Arrow/DataFusion code resists
  `RecordBatch -> Vec<RowStruct>` until the very edge and pushes compute down.
  `skeet-store` materialises early and loops in Rust (§3.3/3.4). Given the data
  volumes here that's a *reasonable* simplicity trade — your own rule says "prefer
  simple, readable techniques until profiling identifies a real hotspot," and the
  scores cache shows you already optimise where it counts. The note is just to
  keep paging on the radar as the table grows.

Net: structurally this is **closer to "a tidy application data layer" than to
"a database engine,"** which is the right target for what it is. The divergences
from engine-style code (early materialisation, one wrapper type) are mostly
defensible simplicity choices; the ones that aren't about volume — string
predicates, upsert atomicity, misplaced trace code, error swallowing — are the
ones worth picking up regardless of scale.

---

## 9. If you only touch a few things

Ordered by value-per-effort:

1. **`get_by_ids` / `get_originals_by_ids` → `HashMap<ImageId, _>`** (§E). Your own
   rule, a caller already hand-rolls the workaround, deletes code at the call site.
2. **`list_scores_for_ids(&[&str])` → `&[ImageId]`**, and add one
   `escape_literal` helper for the remaining `format!` predicates (§3.1). Small,
   closes the only real injection hole, enforces an invariant you're currently
   trusting.
3. **Consolidate upserts on `merge_insert`** (§3.2). `upsert_score` is test-only
   and non-atomic — make it a one-row wrapper over `batch_upsert_scores` so tests
   hit the production path; switch the production `set_*_band` (admin UI) off
   `delete`-then-`add` to close its read-visibility gap.
4. **Extract `tempo` + `trace_analysis` to their own crate** (§2.1). Sheds 13% of
   LOC and several deps from the store.
5. **Drop `#[instrument]` from per-row helpers** (§6) and **delete `InvalidUri`**
   (§4). Trivial.
6. **Replace `ValidationFailed(e.to_string())` / `InvalidZone(String)` with
   `#[from]`/`#[source]` variants** (§4). Consistency with the rest of the enum.

Design-level, higher effort, in rough order of payoff: **make the table fields
private and force cross-table access through methods** (§A — the lever for
everything else), then a **read/write capability split** (§B.3, encodes a safety
rule you enforce at runtime today), a **generic `AppraisalTable<K>`** (§D, removes
a duplicated interface), **named result types** for the `(Score, ModelVersion)` /
scored-summary tuples (§E), a **`TableName` enum** (§E), and a **declarative
`TableSpec`** to collapse `open()` (§7/§C). Plus, as the images table grows,
ordered+limited paging via the lance `Scanner` (§3.4).

---

## 10. Migration paths: DataFusion-direct vs Iceberg catalog

Two very different targets got bundled in the question; they have *opposite*
cost profiles, so separate them.

### 10.1 First, what the current design does to migratability
There is **no seam** (§A): `pub(crate)` tables, zero domain traits, every method
speaks `lancedb` directly (`self.images_table.query().only_if(…).execute()`,
`merge_insert`, hand-built `RecordBatch`, `lance::WriteParams`). By the textbook
measure that's "hard to migrate — nothing to swap behind." But that measure
assumes the goal is *replace the backend behind a trait*, which is the wrong frame
for both targets here. What actually matters is how portable each **layer** is:

| Layer | Portable to DataFusion-direct? | Portable to Iceberg? |
|---|---|---|
| Arrow schemas (`schema.rs`) + `RecordBatch` decode (`stored.rs`, `scores.rs`) | **Yes, unchanged** — it's already Arrow | Yes, unchanged (Iceberg is Arrow-friendly too) |
| Filter construction (string predicates) | Rewrite to `Expr`/SQL — **but you want this anyway** | Rewrite to `Expr`/SQL |
| Upsert (`merge_insert`), versioning (`table.version()`), scalar indices, compaction (`optimize`), the R2 metrics wrapper | **Stay** (same engine underneath) | **All need Iceberg equivalents** — different transaction/snapshot/index/compaction model |

The decode layer — the bulk of the fiddly code — is backend-agnostic already.
The lock-in is concentrated in writes, indices, and maintenance.

### 10.2 DataFusion-direct: short, additive, partly already underneath you
`lancedb 0.27.2` and `lance 4.0.0` **are DataFusion applications**: lancedb
depends on the whole `datafusion-*` stack and implements `TableProvider` with
exact filter pushdown (`table/datafusion.rs:184,251`); `lance` exposes
`LanceTableProvider` (`datafusion/dataframe.rs:39`) and a SQL entry point
(`dataset/sql.rs`). So "use DataFusion more directly" is **not a migration** — it's
dropping one layer to the engine you're already running, and it can be done
**per-method, incrementally, with the data staying in Lance**:

- Register a table's `Dataset` as a `LanceTableProvider` in a `SessionContext`,
  then run `SELECT … ORDER BY discovered_at DESC LIMIT n` (fixes paging, §3.4) or
  `SELECT model_version, COUNT(*) … GROUP BY model_version` (fixes the in-Rust
  counts, §3.3). The lance `Scanner` already has `count_star`/`count`/`aggregate`
  (`scanner.rs:595,608,1233`) if you don't want full SQL.
- The DataFrame builder (`col("image_id").eq(lit(id))`) is the typed filter path
  that closes the string-interpolation hole (§3.1) at the same time.

The current trait design barely affects this, *because you're not swapping an
implementation* — you're calling a lower layer of the same stack. The refactors
that would make it tidy are exactly the ones already recommended: centralise query
construction behind one seam (§3.1/§E) so "lancedb `Query` vs `SessionContext`
SQL" is a per-method choice in one place, not scattered. **Verdict: low cost,
high payoff, no architectural commitment — the pragmatic path, and it resolves
several existing findings as a side effect.**

### 10.3 Iceberg: expensive *and* probably the wrong target for this workload
Iceberg is absent from the dependency tree, and adopting it is a different class
of change:

- **It's a data-format migration, not an API swap.** Iceberg is Parquet + manifest
  metadata; Lance is its own format. Moving means rewriting all data and replacing
  the lancedb-managed index/version/`merge_insert`/compaction machinery with
  Iceberg's snapshot/commit model. The ecosystem is ready in principle
  ([iceberg-rust](https://github.com/apache/iceberg-rust),
  [`datafusion_iceberg`](https://lib.rs/crates/datafusion_iceberg) give
  `TableProvider`/`CatalogProvider` + maturing writes), so it's *possible* — the
  question is whether it's *wanted*.
- **Workload mismatch.** Bobby does **point lookups by content-hash `image_id`**
  and stores **~2 MB PNG blobs per row**. Iceberg/Parquet is tuned for large
  analytical scans with file-level min/max pruning — it has **no scalar point
  index** (the thing your `image_id`/`discovered_at` B-tree indices give you), and
  inline multi-MB blobs in Parquet row groups are awkward. Lance is *designed* for
  random access + blobs + indices. On the metrics that matter here, Iceberg is a
  likely **downgrade**.
- **Only the catalog is genuinely attractive** — open, multi-engine
  (Spark/Trino/DuckDB/Snowflake reading the same tables), governance, real
  namespaces. But your "catalog" is 5 tables whose versioning + prod/staging split
  is already solved by naming conventions (`docs/versioning.md`), and no external
  engine needs to read them today.

How the trait design affects *this* one: even a perfect `trait ImageReader`/
writer split (§B.3) — which is the *only* part that would help, by letting you
stand up an Iceberg impl side-by-side and dual-read during a cutover — cannot
paper over the format/index/transaction differences. So the coupling is **not**
the main obstacle; the format and workload fit are.

### 10.4 If the *motivation* is "an open catalog," there's a nearer answer
Lance SDK **1.0.0** (Dec 2025) shipped **Lance Namespace**, explicitly versioned
"following an upgrade strategy similar to the **Iceberg REST Catalog spec**," and
`lancedb 0.29/0.30` expose namespace management on the connection. So the catalog
abstraction that makes Iceberg tempting is now available **without leaving Lance**
— and it would be a natural structural home for the prod/staging split you
currently encode in table names. The format-stability guarantee in 1.0 ("breaking
changes will not require rewriting existing Lance data") also means staying on
Lance is low-risk.

**Bottom line:** adopt DataFusion-direct incrementally (cheap, additive, already
underneath you, fixes findings); treat Iceberg as a "only if a concrete
multi-engine/open-governance requirement appears, and evaluate Lance Namespace
first" option, not a default direction. The current lack of a trait seam is
*irrelevant* to the DataFusion path and *not the binding constraint* on the
Iceberg path.

---

## 11. LanceDB upgrade: 0.27.2 → 0.30.0 — features worth taking

You're on `lancedb 0.27.2` (31 Mar 2026); latest stable is **`0.30.0` (28 May
2026)**, with `0.29.0` in between. Lance core is **1.0.0**. Two minors back. What
the upgrade (or your *already-pinned* `lance 4.0.0`) unlocks, tied to findings:

| Feature | Where | Resolves / enables |
|---|---|---|
| **`order_by` on the high-level `Query` builder** | **0.30.0, confirmed in Rust source** (`query.rs:519`, `fn order_by(self, Option<Vec<ColumnOrdering>>)`; absent in 0.27.2) | Paging pushdown (§3.4) **without** dropping to the `Scanner` — `order_by(discovered_at desc).limit(n)` directly |
| **DataFusion `Expr` predicates for `merge_insert` + deletes** | 0.30 (`only_if_expr` already in 0.27.2) | Typed filter path everywhere → closes the string-interpolation/injection hole (§3.1) and the upsert predicates |
| **Namespace / catalog management on the connection** | 0.29/0.30 + Lance Namespace (1.0) | A structural home for the prod/staging split (§10.4) instead of name conventions |
| **Unenforced primary keys** | 0.30 | Simplifies `image_id`/score key handling; pairs with `merge_insert` (§3.2) |
| **Blob v2 column + lazy `BlobFile` / `take_blobs`** | **`lance 4.0.0` only** (`blob.rs`, `dataset.rs:1440`); **not surfaced by lancedb's high-level API even in 0.30** (verified — no blob handling in `lancedb-0.30.0/src`) | The 2 MB PNGs (see expanded note below) |
| **`Scanner` aggregations** (`count_star`/`count`/`aggregate`) + `LanceTableProvider` SQL | **already in `lance 4.0.0`**, reachable via `as_native()` | Push `count_scored_images` / `count_scores_by_model_version` / `unique_skeet_ids` down to the engine (§3.3) |
| IVF_HNSW_FLAT vector index, model-backed FTS tokenizers | 0.29 | Not needed now, but relevant if landmark/semantic scoring ever wants vector or full-text retrieval |

### 11.1 The 2 MB PNGs: a spectrum, and what blob v2 actually is
Lance blob v2 (`lance 4.0.0` `blob.rs`) is a column shaped as
`Struct<data: LargeBinary?, uri: Utf8?>` tagged `lance.blob.v2`, with a
`BlobArrayBuilder` and lazy `Dataset::take_blobs` → `BlobFile`. Crucially each row
is **either inline `data` or an external `uri`**. So the options for the pixels,
cheapest-to-adopt first:
  1. **Status quo** — inline `LargeBinary`. Columnar projection already keeps
     pixels out of summary scans (you `select` without `image`), so the main cost
     is the **compaction-memory hack** in `optimise.rs`.
  2. **Blob v2, inline `data`** — pixels stored/compacted in separate blob files;
     lance's `optimize` has dedicated blob handling (`optimize.rs:437,890`,
     `BlobHandling::AllBinary`), which is what could relieve the hand-tuned
     `target_rows_per_fragment=500`. *Cost:* it's a `Struct` column → an
     `images_v7` schema bump + write/read changes, and it's **lance-dataset-level
     work** (lancedb's high-level `Table` doesn't surface it — verified above).
  3. **Blob v2, external `uri`** — Lance holds a pointer, the PNG lives as its own
     R2 object. Table stays tiny; compaction stops touching pixels entirely.
  4. **Plain external R2 objects + Lance metadata only** — no blob v2 at all;
     simplest mechanically, but you lose Lance's transactional coupling of
     image+metadata and take on orphan/GC management yourself.

Given projection already spares summary reads, the honest framing is: blobs are a
**compaction/storage-layout** improvement (options 2–3), not a read-latency one —
worth it if the compaction memory tuning becomes fragile, not urgent otherwise.

### 11.2 The upgrade is not a drop-in — budget for it
The headline cost, from the `0.30.0` manifest: it pins **`lance = "=7.0.0"`** and
**`arrow 58.0.0` / `datafusion 53.0.0`**. You're on **lance 4.0.0, arrow 57**. So
the bump is **arrow 57 → 58 across the whole workspace** (every crate using
`arrow-array`/`arrow-schema`, plus `image`/encoding touchpoints) and **lance 4 →
7, three major versions** of API churn — not a routine minor bump. The benefits
are real, but this is a project, not an afternoon.

### 11.3 The leverage point: most wins need **no** upgrade
The highest-value technical fixes are reachable on your *current* pins:
  - **Injection-safe filters** — `only_if_expr` is in `0.27.2` and routes through
    `scanner.filter_expr` (structured, no SQL) — §3.1, verified.
  - **Ordered+limited paging** and **agg pushdown** — `lance 4.0.0` `Scanner` has
    `order_by`/`limit`/`count_star`/`aggregate`, reachable via the `as_native()`
    you already call — §3.3/§3.4.
  - **Blob v2** — `lance 4.0.0`, dataset-level — §11.1.

What the `0.30` upgrade *adds* on top is mostly **ergonomics and the catalog**:
`order_by` on the high-level builder (so you needn't drop to the `Scanner`),
**namespaces/Lance Namespace** (the prod/staging home, §10.4), and unenforced PKs.

**Net:** don't gate the fixes on the upgrade — do the injection/paging/agg work now
against `lance 4.0.0` (it's the §10.2 "lean into the DataFusion layer" direction).
Schedule the `0.30` upgrade separately, as the arrow-58 + lance-7 jump it really
is, when the namespace/catalog story (or `order_by` ergonomics) earns the churn.

---

## Sources

- [Is the Repository pattern viable in Rust? — users.rust-lang.org](https://users.rust-lang.org/t/is-the-repository-pattern-a-viable-pattern-in-rust/25030)
- [Code structure for big impls over several files — users.rust-lang.org](https://users.rust-lang.org/t/code-structure-for-big-impl-s-distributed-over-several-files/7785)
- [Building robust business logic with Rust service layers — Leapcell](https://leapcell.io/blog/building-robust-business-logic-with-rust-web-service-layers)
- [rust-unofficial/patterns — design patterns & anti-patterns](https://github.com/rust-unofficial/patterns)
- [LanceDB metadata filtering docs](https://docs.lancedb.com/search/filtering) · [filtering guide](https://lancedb.com/docs/search/filtering/) · [QueryBase (docs.rs)](https://docs.rs/lancedb/latest/lancedb/query/trait.QueryBase.html)
- [Lance read & write](https://lance.org/guide/read_and_write/) · [Lance transactions](https://lance.org/format/table/transaction/) · [Lance PK upsert discussion](https://github.com/lance-format/lance/discussions/3842)
- [Error handling in Rust — Luca Palmieri](https://www.lpalmieri.com/posts/error-handling-rust/) · [thiserror catch-all (#52)](https://github.com/dtolnay/thiserror/issues/52)
- [tracing::instrument tutorial](https://gist.github.com/oliverdaff/d1d5e5bc1baba087b768b89ff82dc3ec) · [Distributed Tracing in Practice, ch.6 — overhead & sampling](https://www.oreilly.com/library/view/distributed-tracing-in/9781492056621/ch06.html)
- [GreptimeDB `table` crate](https://greptimedb.rs/table/index.html) · [Rust database implementations — lib.rs](https://lib.rs/database-implementations)
- Local source verified: `lancedb 0.27.2` `src/query.rs` (`only_if_expr:424`, `QueryFilter:698`, no `order_by`); `lance 4.0.0` `src/dataset/scanner.rs` (`order_by:1598`, `limit:1344`, `count_star:595`, `aggregate:1233`); `lance 4.0.0` `src/datafusion/dataframe.rs` (`LanceTableProvider:39`), `src/blob.rs` + `src/dataset.rs:1440` (`take_blobs`/`BlobFile`); `lancedb 0.27.2` `src/table/datafusion.rs:184` (`impl TableProvider`).
- Migration / latest version: [Announcing Lance SDK 1.0.0 (Dec 2025)](https://www.lancedb.com/blog/announcing-lance-sdk) · [lancedb releases](https://github.com/lancedb/lancedb/releases) · crates.io API (lancedb max stable **0.30.0**, 28 May 2026; 0.27.2 = 31 Mar 2026)
- Iceberg in Rust: [apache/iceberg-rust](https://github.com/apache/iceberg-rust) · [`datafusion_iceberg` (lib.rs)](https://lib.rs/crates/datafusion_iceberg) · [DataFusion `TableProvider`](https://docs.rs/datafusion/latest/datafusion/catalog/trait.TableProvider.html)
