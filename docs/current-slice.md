# Current Slice: 1.0 refactor, review and code minimisation, focussed on remaining crates

#### Focus on longer-term maintainance

Refactor, review and minimisation of code for longer-term maintenance so "I can walk away from this for a while".

* the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising code or lingering cruft/weirdness.
* code is already split into sub-dirs by role (all crates live under `crates/`); the remaining bias is towards best-practice structure *within* each crate, following generally accepted conventions where possible.

The general bias is to refactor towards patterns and structures that are the best practice for what kinds of things each crate is doing.

#### Tasks

* [ ] **Publishing** — `skeet-publish`.** The firehose → classify → score → publish chain.
    * **From the patterns review:** tighten over-broad `pub mod` → `mod` + selective `pub use` (most modules are `pub mod` today). Low-priority; do while already in the crate.
* [ ] **Web services — `skeet-feed`, `skeet-appraise`.** The two HTTP-facing crates (banner/feed + auth-gated appraisals).
    * **From the patterns review:** `skeet-feed/src/feed_config.rs` `did()`/`feed_uri()`/`service_endpoint()` return raw `String` → return domain types (`Did`, etc.). Also tighten over-broad `pub mod` → `mod` + `pub use` in both crates (`skeet-appraise/src/lib.rs` ≈12 `pub mod`; `skeet-feed` most modules `pub mod`) — low-priority, do while touching them.
* [ ] **ML/detection libs, and related parent crate which uses them — `skeet-refine`, `face-detection`, `skin-detection`, `text-detection`.** Model loading/inference wrappers; confirm each model is still documented in `docs/`.
    * **Couple every score with its provenance — stop passing bare `Score`.** The store pass introduced `ModelScore { score, model_version }` (a model score carrying the version that produced it) and threaded it through the store ports/read-models. Extend that principle across the scoring pipeline: wherever a `Score` is coupled to *what produced it*, pass the paired type, not a bare `Score` + a sidecar field. A bare `Score` should appear only where code genuinely operates on scores generically (e.g. numeric comparison/sorting).
        * Audit `skeet-refine` (`tick.rs` `pending_scores: Vec<(ImageId, Score, ModelVersion)>`, `refining.rs`, the train harness) and `shared` (`refine_model.rs`) for `Score` + `ModelVersion` passed separately → `ModelScore`.
        * Extract the appraiser analog: an **`AppraiserScore`** (working name) pairing a manual rating with the `Appraiser` who gave it — the `(Band, Appraiser)` that `Appraisal` already half-models and that `Appraisals::set(id, band, appraiser)` still passes positionally. Decide whether this *is* `Appraisal` or a sibling, and route band+appraiser through it.
        * Net effect: `Score` (and `Band`) flow as raw values only inside generic numeric/ordering code; everywhere they cross a boundary they travel with their provenance.
    * **From the patterns review:** validate the `ModelProvider` constructor (`shared/src/refine_model.rs`) — today it accepts any string (only an `openai()` factory; the open `new` path lets an unknown provider propagate silently). Add a known-set / non-empty check. (Co-located here because this area already touches `refine_model.rs`, though the type lives in `shared`.)
* [ ] **Shared/support libs — `shared`, `bluesky`, `web-support`, `build-support`, `test-support`, `eval`, `observability`.** Cross-crate types and helpers; check `shared`'s types stay pure data (no policy methods). (`observability` holds the trace tooling + `query_plan` extracted from `shared`.)
    * **From the patterns review:**
        * `shared/src/rejection.rs`: `Rejection::FromStr` and `RejectionCategory::FromStr` are still `type Err = String` → add a `ParseRejectionError` enum (the recipe every other NewType uses; `ParseZoneError` already done in the store pass).
        * close `&str` gaps where validated NewTypes already exist: `shared/src/skeet_id.rs` `SkeetId::for_post(did, rkey)` → `&Did`/`&RecordKey`; `bluesky/src/image_url.rs` `bsky_cdn_thumbnail_url(did, cid)` → `&Did`/`&BlueskyCid`; `bluesky/src/post_thread.rs` `blocked_labels` `Vec<String>` → `Vec<Label>`.
        * **Pull the Jetstream transport + record interpretation out of `skeet-prune::firehose` into a `bluesky::firehose` (or `bluesky::jetstream`) module** — `bluesky` is the crate that owns "talking to Bluesky," and this is generic ingress with no pruner domain in it. Move: `connect()` + the endpoint list + compression/timeout consts (returns a raw `JetstreamReceiver`), and the record-interpretation helpers (`extract_images`, `has_excluded_label`, `blob_cid`, `parse_created_at`) — the same family as `post_thread`'s label interpretation. **Leave in the pruner:** `SkeetCandidate`/`ImageCandidate` (pipeline domain, keyed by `SkeetId` — or lift to `shared`), `extract_skeet_candidate` (assembles the pruner's candidate by calling the bluesky helpers), and `download_candidate_images` (operates on the candidate types). Widens `bluesky`'s charter from "AppView client" to "AppView + Jetstream ingress" and pulls in `jetstream-oxide`/`atrium_api`/`fastrand` — update the lib.rs charter doc-comment to match. The cursor param + `backon` wrapping that reshape `connect`'s signature have already landed (firehose-improve slice), so the signature is stable and this is ready to do. The reconnect loop, cursor tracking, and backoff stay in the pruner (consumption-robustness wrapped *around* `connect`).
        * replace `Box<dyn std::error::Error>` with typed `thiserror` variants in `shared/src/lib.rs` `PruneConfig::from_file` and `shared/src/blocklist.rs` `BlocklistConfig::{from_file,save}` — the only library fns not on typed errors.
        * validate the `Purpose` constructor (`eval/src/results.rs`) — it accepts empty strings today.
* [ ] **Metrics exporters — `cloudflare-exporter`, `openai-exporter`.** Confirm both are still wired up and used; delete if obsolete.

> **Patterns assessed and not pursued** (from the deleted patterns review, recorded so they aren't re-raised): TypeState for the `skeet-prune` pipeline assembly (ceremony exceeds the payoff for ~50 lines of linear setup); zero-copy borrowing views (clone-based is right for this throughput + async/channel boundaries); combinator-style filter composition (inline iterator chains are simpler — only pays off for filters built dynamically at runtime).
