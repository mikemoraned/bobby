# Current Slice: 1.0 refactor, review and code minimisation

### Target

Refactor, review and minimisation of code for longer-term maintenance — the "I can walk away from this for a while" payoff slice.

* each crate should have at least one human pass where all code is inspected, and deleted/reworked as needed.
* the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising code or lingering cruft/weirdness.
* split out code into sub-dirs based on role e.g. crates are at top-level in repo, and so should go into a subdir; follow generally accepted conventions where possible.
* refactor `just` rules into more logical chunks, and do a pass to remove any that no-longer make sense.

### Decisions (confirmed 2026-06-16)

* **Crate layout: flat `crates/`.** Move every workspace member under a single `crates/` dir (the common Cargo convention) — `crates/skeet-feed`, `crates/shared`, etc. No role subgrouping inside `crates/`; keep paths shallow. Non-crate role dirs (`docs/`, `config/`, `infra/`, `just/`) stay where they are.
* **Deploy artifacts: `deploy/`.** Group `fly.*.toml`, `Dockerfile.*`, and the `*.env` files under `deploy/`, updating the just recipes (`op run --env-file`, `fly --config`) and Dockerfile `COPY` paths accordingly.

### Tasks: Refactors / Cleanups

#### Directory reorg

* [x] **Move all crates under `crates/`.** Relocate every workspace member into `crates/`, then update: workspace `members` and `[workspace.dependencies]` path entries in the root `Cargo.toml`, each crate's own relative path deps, Dockerfile `COPY` paths (`Dockerfile.fly`, `Dockerfile.cluster`), and any just recipes / config that reference a crate path. `just clippy` + `just test-no-docker` must pass unchanged afterwards.
* [x] **Move deploy artifacts under `deploy/`.** Relocate `fly.*.toml`, `Dockerfile.*`, and the root `*.env` files into `deploy/`, then fix the just recipes that reference them (`op run --env-file`, `fly deploy --config`, `docker build -f`). Verify with a dry-run / `fly config validate` where possible. (`fly config validate` passed for all four configs. Also added `deploy/` to `.dockerignore` since the fly tomls/Dockerfiles aren't needed inside the build context.)

#### Per-crate inspection passes

Each crate gets at least one full human pass: read all code, delete dead code, rework anything surprising, enforce the house rules along the way — `lib.rs` under 300 lines (extract modules if over), no `#[allow(dead_code)]`, no `unwrap`/`expect` in non-test code without a justified allow, and strip comment-rot (slice/phase/PR/task refs — see [[no-slice-phase-refs-in-code-comments]]). Note any non-obvious findings per crate.

* [ ] **Data/store layer — `skeet-store`.** The largest crate (~6.3k LOC); pay attention to module boundaries and the read/write paths. Run `just validate-storage` after.
    * Trivial/Small fixes from the reviews (scoped to `skeet-store` + its `shared` deps):
        * [x] Delete the dead `StoreError::InvalidUri` variant.
        * [x] Drop `#[instrument]` from per-row helpers (`to_summary`, `extract`, `typed_column`, `encode_image_as_png`).
        * [x] Drop the always-`Some` `Option` wrapping `store_wrapper`.
        * [x] `list_scores_for_ids(&[&str])` → `&[ImageId]` — closes the one real SQL-injection hole.
        * [x] `get_by_ids` / `get_originals_by_ids` return `HashMap<ImageId, _>` instead of `Vec` — deletes the hand-rolled map in `skeet-refine/loader.rs`.
        * [x] Typed error variants: `ValidationFailed(String)` → `#[from] shared::InvalidScore`; `InvalidZone(String)` → `#[source]`, which needs `shared`'s `Zone::FromStr` to return a new `ParseZoneError` instead of `type Err = String`.
        * [x] Consolidate upserts on `merge_insert` — make test-only `upsert_score` a one-row wrapper over `batch_upsert_scores`; switch `set_skeet_band`/`set_image_band` off `delete`-then-`add`.
    * [x] Extract the unrelated trace tooling into a new `observability` crate — `tempo` (Grafana Tempo client) + `trace_analysis` + the `trace-summary` bin (~850 LOC / 13%, touching nothing in the store); sheds `reqwest`/`serde_json` from the store's deps. Move the shared `query_plan` type into `shared` (the store's live query-logging keeps using it from there). Larger move — do after the cleanup batch above.
    * **Domain & versioning ports — narrow the wide `SkeetStore` (ports & adapters).** Carve the ~35-method `SkeetStore` god-type into cohesive trait "ports"; `SkeetStore` stays the single concrete adapter implementing them all. Two orthogonal, separately-stageable changes (A then B). Background analysis: `skeet-store-review.md` (★ §A/B/C) and `rust-patterns-review.md` (§1.6). The per-thing carve is chosen over the read/write capability split (review §B.3 — a separate, later option).
        * **Scope guardrails (both stages):**
            * Extract traits *over* the existing `SkeetStore`; **do not** split it into sub-structs — it stays the one concrete impl (the adapter). Keeps this medium and reversible.
            * Consumers depend on the **narrowest** port(s) they use, not the whole type. Choose dispatch per wiring: `impl Trait`/generic bounds for directly-constructed consumers (compile-time, no machinery); `Arc<dyn …>` + `#[async_trait]` (already a workspace dep, cf. `FeedSource`) where the store flows through a DI/extension container.
            * Direct `SkeetStore` consumers: `skeet-prune` + `skeet-refine` (writers + some reads), `skeet-publish` (reader — scored-summary read-model + version snapshot), `skeet-appraise` (reads + appraisal writes). `skeet-feed` does **not** touch `SkeetStore` — it reads Redis via `skeet-publish`'s `FeedSource`/`PublishedImagesSource`.
            * Verify each stage: `just clippy`, `just test-no-docker`, `just validate-storage`.
        * [ ] **Stage A — versioning port (do first; smallest, most isolated).** Target: split the *mechanism* of version-gated lazy refresh (already in `versioned_cache.rs::VersionedCache`) from its *source* — the per-table version token, today Lance-specific (`version.rs::version_snapshot`, `lib.rs::table_versions`/`fragment_counts`, and `scores_table.version()` inside `scores.rs::cached_scores`). Extract a small **store-agnostic** trait that yields a comparable version token per logical table (plus the snapshot/fragment-count surface), so the lazy-update logic no longer depends on LanceDB. `version_snapshot` already returns opaque `Version` tags, so this mostly formalises what exists. Repoint: `scores.rs::cached_scores` (cache gating) and `skeet-publish` (`table_watch`, `publisher.rs` `version_snapshot`).
        * [ ] **Stage B — domain ports.** Target: a narrower, more cohesive domain/entity layer that cuts unnecessary coupling/visibility with the wide `SkeetStore`. Carve by the thing acted on:
            * **Images port** (pruned images): `add`, `get_by_id`, `get_by_ids`, `get_originals_by_ids`, `exists`, `delete_by_id`, `count`, `list_all_summaries`, `list_summaries_page` (`paging.rs`), `unique_skeet_ids`, `list_all_image_ids_by_most_recent`.
            * **Scores port** (refine scores): `upsert_score`, `batch_upsert_scores`, `get_score`, `list_scores_for_ids`, `count_scores_by_model_version`, `count_scored_images`.
            * **Appraisals port**: `set_/get_/clear_skeet_band` + `list_all_skeet_appraisals`, `set_/get_/clear_image_band` + `list_all_image_appraisals`. (Optional impl-side cleanup *beneath* the port: collapse the duplicated skeet/image CRUD into a generic `AppraisalTable<K>` — review §D.)
            * **The one real decision — cross-table read-models.** `list_scored_summaries_by_score`, `list_scored_summaries_published_since`, `list_unscored_image_ids`, and `summarise` (`summary.rs`) all join images+scores and belong to **neither** per-thing port — give them their own small **read-model / "scored view" port** rather than forcing them onto the Scores port. Main consumer: `skeet-publish`.
            * Leave health/maintenance (`health.rs`, `optimise.rs`, `validate`) on the concrete `SkeetStore` — out of scope for the narrowing.
            * Then repoint each consumer at the minimal port(s) it actually uses.
        * **Deferred (not this slice, no concrete driver yet):** Stage C — decompose the Lance/R2 adapter into a small number of composable sub-impls (e.g. a swappable image-encryption codec) for a future "plaintext R2 + encrypted image blobs" migration; this is orthogonal to A/B (the codec lives *below* the ports) and demand-driven. Also deferred: splitting `SkeetStore` into per-table sub-structs, and the read/write capability split (review §B.3).
* [ ] **Processing-pipeline binaries — `skeet-prune`, `skeet-refine`, `skeet-publish`.** The firehose → classify → score → publish chain.
* [ ] **Web services — `skeet-feed`, `skeet-appraise`.** The two HTTP-facing crates (banner/feed + auth-gated appraisals).
* [ ] **ML/detection libs — `face-detection`, `skin-detection`, `text-detection`.** Model loading/inference wrappers; confirm each model is still documented in `docs/`.
* [ ] **Shared/support libs — `shared`, `bluesky`, `web-support`, `build-support`, `test-support`, `eval`.** Cross-crate types and helpers; check `shared`'s types stay pure data (no policy methods).
* [ ] **Metrics exporters — `cloudflare-exporter`, `openai-exporter`.** Confirm both are still wired up and used; delete if obsolete.

#### Justfile pass

* [ ] **Re-chunk and prune the just rules.** Review the root `Justfile` and `just/*.just`: split overgrown files (`store.just` ~125 lines, `cluster.just` ~170 lines) into more logical chunks, fold/relocate misplaced recipes, and delete any recipe that no longer makes sense (obsolete deploys, dead helpers). Keep imports consistent and `just --list` readable.

#### Wrap-up

* [ ] **Capture all changes and verify.** Ensure CLAUDE.md / docs references to moved paths are updated, then run `just clippy`, `just test-no-docker`, and `just mutants-on-diff` clean.

### Tasks: Features

#### Add `quality-1y`, `recency-1y`, `quality-4w`, `recency-4w` and make `quality-4w` the default

* [x] Add `y` (meaning 1 one year window) and `w` (meaning 1 week window) as `Limit` options
* [x] in `skeet-appraise` make discovery of `AvailableFeeds` dynamic i.e. it should automatically discovery feeds by naming convention as opposed to a hard-code list
    * this may require changes in `skeet-publish`
    * Approach: `skeet-publish` advertises its published lists in a redis SET (`v{N}-feed-catalog`) at startup, keyed by `PublishedList` name (`PublishedListCatalog`). `skeet-appraise` drops its `--publish` args and instead discovers feeds from the catalog **on every home render** (`PublishedListCatalogReader`), so feeds published after appraise started up are picked up without a restart. Appraise sorts the discovered feeds quality-first then ascending window, defaulting to `quality-4w` (falling back to the first feed if absent).
* [x] Add `quality-1y`, `recency-1y`, `quality-4w`, `recency-4w` as pregenerated lists
    * Added alongside the existing `recency-48h`/`quality-48h`/`quality-7d` in every place skeet-publish's `--publish` set is configured (k8s deployment, `publish.just`, `local.just`). `quality-7d` was left in place (still selectable in appraise).
* [x] Change default choices for `skeet-feed` to be:
    * `quality-48h` for bluesky feed (skeleton)
    * `quality-4w` for main homepage