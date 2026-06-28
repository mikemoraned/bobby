# Next Slices

## Slice: re-train refine model with a new eval snapshot

### Target

We've now amassed 1292 Image Appraisals so we'd probably benefit from capturing a new eval snapshot, and improved prompt. We've got bigger work planned later (see "try using embeddings for classification/scoring in refine") so we just want to focus this slice on allowing existing OpenAI model to be tuned based on a more up-to-date and complete set of image appraisals.

### Decisions / groundwork

- **The machinery already exists** — this is mostly an operational slice driving the bins built in earlier slices (`capture-appraisals`, `refine-eval`, `sample-costs`, `train`, `promote`) against the grown dataset. The checked-in config registries (`config/eval-splits.toml`, `config/eval-results.toml`, `config/refine.toml`) are the inputs/outputs; each step commits the registry it writes.
- **Stay on the existing OpenAI model.** Train keeps `gpt-4o` as both scorer and rewriter; the only thing changing is the prompt, tuned against a fresher/larger appraisal set. Embeddings and cheaper-model exploration stay out of scope (their own slices).
- **A fresh snapshot needs a fresh baseline.** `train`'s `pick_baseline` selects the best-F1 run carrying the production `model_version` **and the same `split_id`**. A newly captured snapshot has a new `split_id`, so no run qualifies until the deployed model is re-evaluated on it — hence the explicit re-baseline step before training.
- **`default` moves to the new snapshot** (capture's existing behaviour): the old phase-2–4 143-image split keeps its registry entry but loses the label, so `train`/`refine-eval` pick up the fresh snapshot automatically. This supersedes the "frozen 143-image split" the embeddings slice references — that slice should pin the old `split_id` explicitly rather than rely on `default`.

### Tasks

* [ ] **Capture the fresh snapshot.** Run `capture-appraisals` against the shared store to write a new `default` split from all current image appraisals (~1292) into `config/eval-splits.toml`. Sanity-check the train/test counts and per-band stratification look right; commit the registry.
* [ ] **Re-baseline the deployed model on the new split.** Run `refine-eval` with the current production `config/refine.toml` against the new `default` split, appending a baseline run (new `split_id`) to `config/eval-results.toml` — this is what `train` gates against. Commit the log.
* [ ] **Cost pre-flight.** Run `sample-costs` over a small stratified sample to confirm `gpt-4o`'s empirical per-image cost under current prices before committing a training budget.
* [ ] **Train an improved prompt.** Run `train` (`gpt-4o`, gated against the re-baseline run). On acceptance it writes the new prompt/model + `decision_threshold` to `config/refine.toml` and appends the run to the log; on rejection the incumbent stands. A single run is noisy — if the candidate lands borderline against the gate, re-run before trusting it (full variance-aware gating is the later "improving prune and refine quality" slice).
* [ ] **Promote + deploy** (only if a candidate is accepted). Repoint the `production` label via the `promote` bin and do the k8s image cutover (per `docs/versioning.md` + `docs/remote-setup.md`) so the new model serves in prod. If rejected, record that the fresh snapshot reaffirmed the incumbent and skip the flip.
* [ ] **Verify + document.** `just clippy` + `just test-no-docker`. Record the snapshot `split_id`, baseline run-id, candidate run-id, and outcome (accepted+promoted, or rejected) so the comparison is reproducible.


## Slice: drain the pipeline on SIGTERM (close the restart in-flight loss)

### Target

The firehose slice made *reconnect* gaps lossless (cursor) but did nothing for *restart* loss. Shutdown today is purely **reactive**: stages unwind only when a channel closes (`docs/skeet-prune-pipeline.md` "Shutdown"). On a k8s redeploy (SIGTERM → SIGKILL after the grace period) the pruner is hard-killed with up to ~16+16+100 buffered items plus in-flight download/classify work across the three channels — all silently lost.

This is the other half of restart-loss the firehose slice deliberately deferred (the cursor's in-memory-only choice "accepts the restart gap as lost"; this slice narrows that gap to "drain what's already in the pipeline").

### Decisions / groundwork

- **The shared `CancellationToken` already exists** — the firehose slice introduced it as the closed-downstream seam (`pipeline.rs` `forward`/`recv`). This slice adds the *deliberate* trip, not a from-scratch shutdown mechanism.
- The shape: a SIGTERM handler trips the token to stop the **source** (firehose recv loop), then the stages are supervised (e.g. a `JoinSet`) and awaited so the bounded channels **drain into the idempotent store** before exit. Draining is safe precisely because the sink is content-hash idempotent.
- Compose with the cursor, don't compete: the cursor handles reconnect; this handles graceful restart. Persisting the cursor across restarts stays out of scope (a separate, larger choice).

### Tasks

* [ ] SIGTERM/SIGINT handler that trips the shared `CancellationToken` to stop the firehose source.
* [ ] Supervise the stages (`JoinSet` or equivalent) so `main` awaits their completion rather than only awaiting the sink; on shutdown, let the channels drain into the store before exit, bounded by a drain deadline shorter than the k8s grace period.
* [ ] Update `docs/skeet-prune-pipeline.md` "Shutdown" to describe deliberate drain alongside the reactive close.
* [ ] Verify `just clippy` + `just test-no-docker`. A deterministic loop test is awkward (signal/time-bound); a live SIGTERM smoke-check (observe the channels drain, no lost in-flight items) is the human/CI step.

## Slice: 1.0 refactor, review and code minimisation, focussed on remaining crates

#### Focus on longer-term maintainance

Refactor, review and minimisation of code for longer-term maintenance so "I can walk away from this for a while".

* the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising code or lingering cruft/weirdness.
* split out code into sub-dirs based on role e.g. crates are at top-level in repo, and so should go into a subdir; follow generally accepted conventions where possible.

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
* [ ] **Shared/support libs — `shared`, `bluesky`, `web-support`, `build-support`, `test-support`, `eval`.** Cross-crate types and helpers; check `shared`'s types stay pure data (no policy methods).
    * **From the patterns review:**
        * `shared/src/rejection.rs`: `Rejection::FromStr` and `RejectionCategory::FromStr` are still `type Err = String` → add a `ParseRejectionError` enum (the recipe every other NewType uses; `ParseZoneError` already done in the store pass).
        * close `&str` gaps where validated NewTypes already exist: `shared/src/skeet_id.rs` `SkeetId::for_post(did, rkey)` → `&Did`/`&RecordKey`; `bluesky/src/image_url.rs` `bsky_cdn_thumbnail_url(did, cid)` → `&Did`/`&BlueskyCid`; `bluesky/src/post_thread.rs` `blocked_labels` `Vec<String>` → `Vec<Label>`.
        * **Pull the Jetstream transport + record interpretation out of `skeet-prune::firehose` into a `bluesky::firehose` (or `bluesky::jetstream`) module** — `bluesky` is the crate that owns "talking to Bluesky," and this is generic ingress with no pruner domain in it. Move: `connect()` + the endpoint list + compression/timeout consts (returns a raw `JetstreamReceiver`), and the record-interpretation helpers (`extract_images`, `has_excluded_label`, `blob_cid`, `parse_created_at`) — the same family as `post_thread`'s label interpretation. **Leave in the pruner:** `SkeetCandidate`/`ImageCandidate` (pipeline domain, keyed by `SkeetId` — or lift to `shared`), `extract_skeet_candidate` (assembles the pruner's candidate by calling the bluesky helpers), and `download_candidate_images` (operates on the candidate types). Widens `bluesky`'s charter from "AppView client" to "AppView + Jetstream ingress" and pulls in `jetstream-oxide`/`atrium_api`/`fastrand` — update the lib.rs charter doc-comment to match. **Do this only after the firehose-improve slice's Groups 2/3 land** — the cursor param + `backon` wrapping reshape `connect`'s signature, so move it once stable. The reconnect loop, cursor tracking, and backoff stay in the pruner (consumption-robustness wrapped *around* `connect`).
        * replace `Box<dyn std::error::Error>` with typed `thiserror` variants in `shared/src/lib.rs` `PruneConfig::from_file` and `shared/src/blocklist.rs` `BlocklistConfig::{from_file,save}` — the only library fns not on typed errors.
        * validate the `Purpose` constructor (`eval/src/results.rs`) — it accepts empty strings today.
* [ ] **Metrics exporters — `cloudflare-exporter`, `openai-exporter`.** Confirm both are still wired up and used; delete if obsolete.

> **Patterns assessed and not pursued** (from the deleted patterns review, recorded so they aren't re-raised): TypeState for the `skeet-prune` pipeline assembly (ceremony exceeds the payoff for ~50 lines of linear setup); zero-copy borrowing views (clone-based is right for this throughput + async/channel boundaries); combinator-style filter composition (inline iterator chains are simpler — only pays off for filters built dynamically at runtime).

## Slice: `skeet-store` engine & storage scaling

These were identified in the "1.0 refactor, review and code minimisation, focussed on skeet-store" slice but deliberately deferred as too large for that slice. Each is gated 
on data scale, needs a dependency upgrade, or is strategic. Captured here so the analysis survives the slices summarisation.

Analysis assumed these pins: `lancedb 0.27.2` / `lance 4.0.0` / `arrow 57`. None of the no-upgrade items below need the lance 0.30 bump.

### Engine pushdown (no upgrade; gated on scale)

Several read paths materialise whole tables into Rust and compute there — fine at
current volume, revisit when the images table is big enough to hurt.

* [ ] **Paging pushdown.** `list_summaries_page` pushes the `discovered_at < cursor`
  filter down but then collects *every* matching row, sorts in memory, and truncates
  to `limit` — O(rows-before-cursor), not O(limit). The high-level `lancedb 0.27.2`
  `Query` builder has `limit`/`offset` but **no `order_by`**, so the in-memory sort is
  a workaround for a missing method. Fix via the `lance 4.0.0` `Scanner` (`as_native()`,
  already used elsewhere): `order_by(discovered_at desc)` + `limit(n+1)`, letting the
  `discovered_at` scalar index do the work.
* [ ] **Aggregation/distinct pushdown.** `count_scored_images`,
  `count_scores_by_model_version`, `unique_skeet_ids`, and the in-memory sort in
  `list_all_image_ids_by_most_recent` scan + compute in Rust. Push down via the lance
  `Scanner` `count_star`/`count`/`aggregate` (or DataFusion SQL). Leave the
  version-gated `cached_scores` full-scan as-is — it's cached and the known-versions
  filter is awkward as pushdown.

### DataFusion-direct (the pragmatic path; subsumes the above)

* [ ] `lancedb`/`lance` *are* DataFusion apps; `lance` exposes `LanceTableProvider` + a
  SQL entry point. "Use DataFusion directly" is **not** a migration — register a
  table's `Dataset` as a `LanceTableProvider` in a `SessionContext` and run
  `SELECT … ORDER BY … LIMIT n` / `GROUP BY` for the complex reads, per-method,
  incrementally, data staying in Lance. The typed-`only_if_expr` work from the store
  pass is the groundwork (one query-construction seam in `adapters/lance/query.rs`).
  Low cost, additive, no architectural commitment; resolves the pushdown items as a
  side effect.

### LanceDB 0.30 upgrade (a project, not an afternoon)

* [ ] `lancedb 0.30.0` pins `lance =7.0.0` + `arrow 58` + `datafusion 53` → a
  **workspace-wide `arrow 57→58` bump and `lance 4→7` (three majors)**. Budget for it;
  do it *after* the no-upgrade pushdown work. What it buys: `order_by` on the high-level
  `Query` builder (paging without dropping to `Scanner`), DataFusion `Expr` predicates
  for `merge_insert`/deletes, **Lance Namespace** (catalog), and unenforced primary keys.

### Blob v2 for the 2 MB PNGs (storage/compaction; needs `images_v7`)

* [ ] Images are inline `LargeBinary` today; columnar projection already keeps pixels
  out of summary scans, so this is a **compaction/storage-layout** win (could relieve
  the hand-tuned `target_rows_per_fragment=500` memory hack in maintenance), **not** a
  read-latency one. Lance 4.0.0 blob v2 is a `Struct<data: LargeBinary?, uri: Utf8?>`
  column (inline `data` *or* external `uri`) with dedicated `optimize` blob handling —
  but it's **lance-dataset-level** (lancedb's high-level `Table` doesn't surface it even
  in 0.30) and needs an `images_v7` schema bump. Worth it only if the compaction memory
  tuning becomes fragile.

### Lance Namespace as the prod/staging home (with/after 0.30)

* [ ] The prod/staging split is currently table-name conventions (`docs/versioning.md`).
  Lance Namespace (SDK 1.0, exposed on the connection in lancedb 0.29/0.30, versioned
  like the Iceberg REST Catalog spec) is a structural home for it — adopt alongside the
  0.30 upgrade if the naming convention starts to chafe.

### Read/write capability split (type-level safety; consciously deferred)

* [ ] The store-pass carve was by-thing (Images/Scores/…); the review's other option
  was a **read/write capability split** — a read-only interface (`trait ImageReader`/
  `ScoreReader`, or a `ReadStore`) that reader-side consumers depend on so they *cannot*
  call `add`/`upsert`/`delete`. This makes the prod/staging "readers are covariant; never
  run a staging *writer* against the shared store" rule — today a runtime
  `--allow-shared-store-write` flag — a **compile-time** fact. The concrete `SkeetStore`
  implements both halves; writers take the full type. Layers over the existing ports.

### Strong-consistency read tuning (small)

* [ ] Every read uses `read_consistency_interval(Duration::ZERO)` (re-checks the manifest
  each op; every Strong read pays a growing R2 LIST, bounded by hourly manifest pruning).
  A deliberate correctness choice — but read-mostly paths (feed serving) might tolerate a
  few seconds of staleness for fewer R2 LISTs. Consider surfacing the interval through
  `StoreArgs` per-CLI.

> Iceberg was considered and rejected as a storage backend — that durable decision
> now lives in `docs/architecture.md` (Constraints / Technology Choices), not here.

## Slice: try using embeddings for classification/scoring in refine

### Target

Refine currently routes every image through an LLM scorer — expensive, slow, and the prompt is the thing we're trying to optimise. Alternative worth measuring: embed each image once with a pre-trained vision model and learn a linear classifier on the embeddings. The embedding does the heavy lifting (visually/semantically similar → close in embedding space), so a linear SVM or logistic regression on top usually recovers a calibrated good/bad score that maps onto Low/MedLow/MedHigh/High bands. Inference drops from ~seconds and per-call cost to milliseconds and free; the prompt-optimisation problem disappears.

- **Deterministic — kills the variance problem.** A frozen embedding plus a seeded linear classifier produces a reproducible score: no temperature noise, no rewriter stochasticity, none of the 0.696–0.870 run-to-run recall spread that made the phase-4 gate hard to read. The re-run-N-times-and-take-a-confidence-interval machinery the LLM path needs just goes away.
- **Embed once, store forever.** Compute each image's embedding a single time and persist it (a new column/table in the lance store); re-scoring under a new classifier is then milliseconds and zero API calls. Contrast the LLM path, where re-scoring the ~34k stored images under a new model is exactly what CrashLoopBackOff'd live-refine (current-slice backfill incident) — here, retraining the classifier and re-scoring everything is trivial.
- **Embedding model matters more than the classifier.** [OpenCLIP](https://github.com/mlfoundations/open_clip) or [SigLIP](https://huggingface.co/docs/transformers/en/model_doc/siglip) for broad visual-semantic; [DinoV2](https://github.com/facebookresearch/dinov2) for pure-visual fine-grained. If the embedding can't see the distinction, nothing downstream recovers it.
- **Calibrated probabilities are the point, not a bonus.** A previous slice showed the cheap LLMs discriminated fine but were mis-calibrated — their scores didn't reach the 0.800-precision operating point without dumping recall. A learned head that emits calibrated 0–1 probabilities natively attacks exactly that binding constraint.
- **Classifier choices.** Logistic regression via [`linfa-logistic`](https://crates.io/crates/linfa-logistic) gives calibrated 0–1 probabilities natively (preferred); linear SVM via [`linfa-svm`](https://crates.io/crates/linfa-svm) + Platt scaling is the alternative; kNN against curated prototypes is a no-training baseline; one-class SVM / SVDD if "bad" is too diffuse to label well.
- **Runtime in Rust.** Embeddings via [`ort`](https://crates.io/crates/ort) (same dep we'd add for the skin-detection slice) or [`candle`](https://github.com/huggingface/candle) for pure-Rust. CPU-only on hetzner (no GPU) — CLIP/SigLIP base is ~tens-of-ms/image, fine at current firehose volume and folds into the fixed monthly cluster cost.
- **Tradeoff + hybrid.** Gives up controllability over *which* aspect of similarity matters; if good/bad hinges on tone/intent the embedding can't see, the LLM scorer still wins. So treat it as the primary gate with the LLM kept for borderline cases — which is also the safe cost bet: if the classifier confidently decides ~80% of images and only the ~20% near the boundary escalate to the LLM, that's an ~80% cost cut without betting everything on full replacement.

### Phase 1: decisive offline experiment (cheap to falsify)

Retire the central risk in an afternoon before building anything, reusing the slice-16 `eval` crate end-to-end:

* [ ] Embed the ~685 appraised images with 2–3 candidate models (e.g. SigLIP, OpenCLIP, DINOv2); cache embeddings to disk/store so every later step is instant
* [ ] Train `linfa-logistic` (cross-validated) on the **same frozen 143-image split** used in phases 2–4, labels from `Band::is_visible_in_feed()`
* [ ] Evaluate on the held-out test set and compare **recall-at-pinned-precision** against the deployed LLM baseline (0.870 @ P=0.800) — same gate as phase 4, so directly comparable
* [ ] Caveat: the split has only ~88 positive training examples (~16%), thin for a learned head — if logreg underperforms, try kNN/one-class before concluding the embedding can't see the distinction. This is where the label-growth bullet (refine slice) pays off most.

## Slice: dynamic social-media preview image for the feed

### Target

A [social media preview image](https://support.metropublisher.com/hc/en-us/articles/31523564070420-Preview-Image-Settings-for-Social-Media) for `bobby.houseofmoran.io` which can be shown on facebook, twitter etc.

* this should be calculated dynamically based on the same `quality-7d` content, and cached using the same last-modified caching from elsewhere.
* We can use something like the layout algorithms used in [linzer](https://github.com/mikemoraned/geo/blob/main/apps/linzer/backend/layout/src/bin/layout.rs) e.g. `Guillotine` from the `binpack2d` crate.

This is a genuine server-side image-composition feature (compose a montage, render it, wire up the OG/Twitter meta tags, cache it), which is why it's its own slice rather than part of the 1.0 feed polish.

## Slice: passkey + fingerprint-allowlist auth for `skeet-appraise`

Replace GitHub OAuth on public `bobby-appraisals.houseofmoran.io` with passkey (WebAuthn) auth where **identity is an allowlisted credential fingerprint in config** — no IdP, no sessions, no redis. The server stores only public-key fingerprints (a few config lines, the analogue of `BOBBY_ADMIN_USERS`); the private key lives in the device's secure hardware, OS-synced across my devices, so there's no client-side key custody. Login is a challenge-response the OS prompt handles (Face/Touch ID). The core is a `--auth-mode`-selected verify-against-an-allowlist check, with `passkey` as one provider arm alongside `github` and `local-admin`.

Run passkey alongside GitHub OAuth, verify, cut over, then rip the OAuth/session/redis stack out. Worktrees keep `--auth-mode local-admin` (reachable over the tailnet) throughout and never touch passkeys.

### Target

* Public prod is **default-deny**: no page, route, API, or image byte reachable without a valid passkey session — protects *access*, not just adding appraisals.
* Identity = a copy-pasteable fingerprint I add to an allowlist in config. Add a device = paste a line; revoke = delete a line + redeploy.
* Enrollment is scriptable: a short pairing code shown on the device also lands in the server logs on the same line as the credential, so `tail fly logs → grep code → extract credential → append` is a one-command `just enroll-device <code> <label>`.
* End with **no GitHub OAuth app, no `/auth/*` routes, no session middleware, no redis**.

### Decisions / constraints

* **Default-deny at the root router.** One middleware wraps everything; the public surface is an explicit small allowlist (login page + the two ceremony endpoints + health). Everything else — assets and image bytes included — needs a valid signed cookie. Allowlisting the public routes (not annotating protected ones) means a new route can't ship unguarded.
* **Close the image side-channel.** Serve images through the authed app (or short-lived presigned URLs), never as public R2 links. SSE-C already closes this; assert it with a test (unauthenticated image fetch → `401`).
* **Allowlist the full fingerprint, never a prefix** (a truncated one is grindable). The fingerprint *identifies*; the full key in the assertion is what the signature is *verified* against. Both must pass: `hash(key) ∈ allowlist` **and** signature valid — the signature check is load-bearing since the public key is public.
* **Enroll-then-bless.** A passkey's public key is minted *during* registration, so the ceremony runs first (harmless — it grants nothing), the server logs the fingerprint, and I paste it into the allowlist to bless it. An unblessed credential is inert, so the ceremony route needs no privileged gating.
* **Pairing code is a correlation handle, not a credential** — it only lets me pick the right log line while holding the device; no private key, no deploy access, so it's safe to keep human-short.
* **WebAuthn specifics:** `rp_id` is host-bound (a prod passkey won't work on a `.ts.net` host — fine, worktrees use `local-admin`); HTTPS required (fly TLS in prod, localhost in the spike); use [`webauthn-rs`](https://crates.io/crates/webauthn-rs) for the crypto — the only logic I own is the allowlist check; recovery = enroll two devices + deploy access as the re-bless path (no lockout cliff).

### Tasks

Spike first, then groups A–E. Run alongside GitHub OAuth, verify, cut over, clean up. Local dev keeps `--local-admin`.

#### Spike (do this first): prove the `webauthn-rs` ceremony + allowlist-verify path on `localhost`

* [ ] Minimal axum harness with register + authenticate ceremony endpoints and an in-memory fingerprint allowlist.
* [ ] Verify, in order: registration yields a public key (print its fingerprint); an *unblessed* credential fails auth (`403`); after adding the fingerprint, auth succeeds and sets a signed cookie; a second device enrolls independently.
* [ ] **Decide credential-id vs public-key-fingerprint** as the allowlist string (credential-id = zero extra hashing; public-key fingerprint = reusable later for signed-appraisal provenance) — group A depends on it. Then tear the harness down.

#### A. Credential identity + allowlist

* [ ] **`Appraiser::Passkey { fingerprint }`** in `shared/src/appraiser.rs`: `provider:identifier` parse/display (`passkey:SHA256:…`), validated constructor, round-trip + unknown-provider tests. Mirrors `GitHub`/`LocalAdmin`.
* [ ] **Allowlist config**: `(fingerprint, label)` lines, 1Password-backed (same shape as `BOBBY_ADMIN_USERS`); required in `passkey` mode (startup fails if empty).
* [ ] **`--auth-mode passkey`** alongside `tailscale`/`github`/`local-admin`; only this mode runs the ceremony + cookie path.

#### B. Default-deny middleware + ceremony endpoints

* [ ] **Root-router auth layer**: validates the signed cookie; only the public allowlist is reachable unauthenticated. Test a dummy route is denied by default.
* [ ] **Ceremony routes** via `webauthn-rs`; on success, verify signature against the presented key **and** `hash(key) ∈ allowlist`, then set a stateless signed cookie (no redis).
* [ ] **Minimal, content-free login page** (single "Sign in" button); rate-limit the ceremony endpoints.
* [ ] **Test**: valid passkey → `Appraiser::Passkey` + appraisal round-trips; unblessed/absent → denied; other modes unaffected.

#### C. Pairing-code enrollment + scripting

* [ ] Client shows a pairing code (~6–8 base32 chars) and sends it with the registration finish.
* [ ] **One structured log line** correlating both: `enroll pairing_id=K7QF2M credential=passkey:SHA256:… ua="…"` (same line — don't split). Log only public fingerprints, never the challenge/assertion. Expire codes after a few minutes.
* [ ] **`just enroll-device <code> <label>`**: grep logs for the code, extract `credential=`, **stage** the line for me to commit + redeploy (never auto-write from a log scrape — logs are an injection surface; the deploy is the gate).

#### D. Close image / asset access

* [ ] Route image serving through auth (or short-lived presigned URLs) — no public R2 links.
* [ ] Test an unauthenticated image/asset fetch `401`s.

#### E. Parallel run, cut over, cleanup

* [ ] **Parallel run**: passkey mode alongside the OAuth site; enroll laptop + phone via the pairing flow; confirm appraisals set/clear and homepage + admin paging work.
* [ ] **Cut over** to passkey (on whichever host carries prod — fly with TLS).
* [ ] **Rip out the dead stack**: delete `auth.rs`, `auth_config.rs` (`OAuthConfig`), the `/auth/{login,callback,logout}` routes, the session middleware + `deadpool-redis` dep, the `BOBBY_GITHUB_*` / `BOBBY_SESSION_SECRET` / sessions-redis config + 1Password items. Drop the `github` arm (leaving `passkey` + `local-admin`). Remove the GitHub OAuth app.
* [ ] **Verify**: `just clippy`, `just test-no-docker`; builds without oauth/session/redis deps (`cargo machete` confirms); relocated integration tests pass.
* [ ] Update docs (`docs/architecture.md`, auth notes) for passkey + fingerprint-allowlist identity.

## Slice: improving prune and refine quality

### Target: prune

I'm still seeing some examples (e.g. examples/v2:de210c2970ed76cf79c27d8cd557214a.png) where the text-detection should ideally be excluding them. I think we can exclude these images by looking at overlap between the text bounding boxes and the 3x3 grid of zones and looking at some features:
1. what %-age of a Zone is taken up by text-boxes (unioned area)?
2. how many Zones have at least some %-age of text-box area?

We can then exclude any images that have > threshold %-age in any Zone, and > number threshold of Zones.

### Target: refine

Ways to improve refine quality and cost, distilled from previous "Slice 16 — make costs visible and reduce them" slice.

**Operating-point preference (governs how every candidate is judged).** The precision floor (0.800) is firm — false positives are the user-visible cost in the feed, so dropping below it is never an acceptable trade. Recall, by contrast, is negotiable: a candidate that holds the precision floor at meaningfully lower cost may lose *some* recall and still be worth deploying. So the baseline's 0.870 recall is a target, not a hard bar.

- **Account for training variance.** A single training run is noisy — `gpt-4o` recall spanned 0.696–0.870 across runs, and the deployed baseline sits at the top of that spread. Gate candidates against a distribution (re-run N times, compare on mean and confidence interval) rather than one lucky draw, so "rejected" means genuinely worse rather than just unluckier. The in-loop also overfits to its own per-iteration sample (train F1 climbs while test recall drops), which larger samples or early-stopping would damp; and reasoning models can't run at `temperature=0`, so their scoring is non-deterministic and needs more repeats to compare fairly.

- **Cost from real measurement, not prediction.** Budget-derived sample sizing assumes the `gpt-4o` token profile, which doesn't transfer: the vision-token multiplier made 4o-mini ~2× *more* expensive, and reasoning-token output made gpt-5 +26% despite cheaper input. The fix is to train and evaluate every contender on equal train/test data under one budget and rank them on *real measured* per-item cost, rather than sizing each run from a baseline-derived guess. The `sample_costs` CLI (`skeet-refine/src/bin/sample_costs.rs`, built in the previous slice) is the pre-flight tool for this: run it once over a small stratified sample to get each candidate's empirical min/max/avg per-image cost before committing to long training runs — a 10-image sample would have caught every cost surprise in the phase-4 sweep.

- **Label quality.** Some gate failures may be label noise in the ~685-appraisal set rather than model error. Reviewing misclassified images and growing/cleaning the set would lift the ceiling for every candidate (and means re-capturing the frozen split to re-baseline).

- **Split scorer vs rewriter.** A previous slice used each candidate as both scorer and prompt-rewriter for simplicity. A strong rewriter producing prompts for a cheap scorer may beat one model doing both — worth testing whether the cheap models' recall collapse is the prompts or an inherent capability gap.

- **Calibration, not discrimination, was the binding ceiling.** Every cheap phase-4 candidate *ranked* images well (ROC-AUC at or above the gpt-4o baseline's 0.897) yet failed the gate because their scores sat in the wrong place on the 0–1 scale — nano overconfident (scores piled at the extremes), gpt-5 too conservative (needed a 0.22 threshold) — so none could reach 0.800 precision without dumping recall. The lever is therefore recalibrating an accepted model's scores (Platt/isotonic) or relaxing the gate from a single pinned-precision point to a (P, R) Pareto-frontier comparison — not hunting for a model with better discrimination. (The owner's precision floor of 0.800 is firm regardless, so a more lenient gate alone wouldn't change the outcome — only a candidate calibrated to high recall *at* that floor would.)

### Tasks

#### Tech-debt / bugs

##### Classify retries by HTTP status, not the blanket `Completion(_)` match

The `refine_image_resilient` wrapper's `is_transient` treats **every** `RefineError::Completion(_)` as retryable, so a permanent client error (e.g. the gpt-5 `temperature=0` HTTP 400) is retried 3× per call before falling back — wasted calls and a flood of WARN logs. Only 429, 5xx, and network errors are genuinely transient; a 4xx (other than 429) is permanent and should fail fast. The live trigger (the temperature-0/reasoning-model 400) is already resolved by the per-model `temperature_for`, so nothing is on fire — but any future permanent client error is still mis-retried.

- [ ] Preserve rig's HTTP status on the `RefineError::Completion` variant rather than stringifying the error (today the status is discarded), so retry classification has something reliable to switch on
- [ ] Rewrite `is_transient` to retry only on 429, 5xx, and network/transport errors; treat other 4xx as permanent (fail fast, no retry, no fallback churn)
- [ ] Avoid string-matching `"400"` in the error message — it's fragile; switch on the preserved status class instead
- [ ] Add unit tests: a permanent 4xx is not retried; a 429/5xx/network error is retried up to the bound

...

## Slice: reducing unintentional bias

### Target

The current skin-detection method in `lib.rs` (Kovac/Peer/Solina 2003 RGB rules + a YCbCr box) is biased toward lighter skin tones. Replace it with a method that performs more fairly across the Fitzpatrick scale, and add tests that would catch this kind of regression in future.

### Tasks

#### Document and demonstrate the current bias
- [ ] Write up the specific lines in `is_skin_pixel` that exclude darker skin:
    - [ ] `r <= 95.0` reject — eliminates much dark brown skin outright
    - [ ] `g <= 40.0` / `b <= 20.0` rejects — fail in shadow and on very dark skin
    - [ ] `(r - g).abs() <= 15.0` reject — absolute R−G gap shrinks at lower intensities even when the ratio is preserved
    - [ ] `max - min <= 15.0` reject — same low-intensity compression problem
    - [ ] note that the YCbCr box is the least-biased part but is ANDed with the RGB gate, so the RGB rules dominate failures
- [ ] Add failing unit tests with known dark-skin RGB samples (e.g. `(80, 50, 35)`, `(60, 40, 30)`, `(110, 75, 55)`) asserting they should be classified as skin — these should fail against the current implementation and pass against the replacement
- [ ] Assemble a small evaluation set of face images spanning Fitzpatrick I–VI and measure per-bucket true-positive rate before and after the change

#### Pick a less-biased method
- [ ] Evaluate options in roughly increasing order of effort:
    - [ ] **CbCr-only elliptical region** (Hsu, Abdel-Mottaleb & Jain, 2002) — drop the RGB gate entirely, fit an ellipse in CbCr space rather than an axis-aligned box. Small code change, big fairness improvement.
    - [ ] **HSV or normalised-rgb thresholds** — hue ≈ [0°, 50°] with moderate saturation and *any* value; removes the luminance dependency that hurts dark skin
    - [ ] **Jones & Rehg statistical skin model** (1999) — Bayesian histogram trained on a large diverse pixel set, runtime is a 3D lookup table, still the standard classical baseline
    - [ ] **Modern ML model trained on a diverse dataset** — anything evaluated on Fitzpatrick 17k or trained on FSD/ECU/Pratheepan; highest accuracy, adds a dependency

#### Rust ecosystem options
- [ ] **Pure-Rust / classical (no new heavy deps).** There is no dedicated "less-biased skin detector" crate on crates.io — closest neighbours are face-detection crates, not skin segmentation. So this path is hand-rolled on top of the existing `image` crate:
    - [ ] Implement a CbCr-ellipse or HSV test directly in `lib.rs`
    - [ ] Optionally fit a Jones-and-Rehg histogram offline against the [UCI Skin Segmentation dataset](https://archive.ics.uci.edu/ml/datasets/skin+segmentation) (built from face images "of diversity of age, gender, and race") and ship the resulting lookup table as a `.bin` in the repo
- [ ] **ML model via ONNX.** The standard route for running pretrained vision models from Rust is the [`ort`](https://crates.io/crates/ort) crate (ONNX Runtime bindings). [`rust-faces`](https://crates.io/crates/rust-faces) is a good template for how to wire an ONNX model into a Rust API similar to our `detect_skin` signature.
- [ ] **Candidate model:** [samhaswon/skin_segmentation](https://github.com/samhaswon/skin_segmentation) on GitHub — a benchmark/training repo with ONNX exports of several skin-segmentation models (BiRefNet, U²-Net variants, etc.). The author explicitly built the training set "to maximize diversity of scene, lighting, and skin appearance" with augmentations designed so the model isn't dependent on lighting or camera settings. Caveat: the heaviest BiRefNet variant uses ~40 GB RAM through onnxruntime, so pick one of the smaller CNN models.
- [ ] **Background reading** for evaluation methodology and fairness framing:
    - [ ] [Fitzpatrick 17k](https://github.com/mattgroh/fitzpatrick17k) (Groh et al., 2021) — standard fairness benchmark
    - [ ] [Bencevic et al. (2024)](https://www.sciencedirect.com/science/article/pii/S0169260724000403) — quantifies the same bias pattern across U-Net-based skin segmentation models

#### Recommended path
- [ ] **Step 1 — cheap win.** Replace the RGB+CbCr-box rules with either a CbCr ellipse or a Jones-and-Rehg histogram trained on the UCI dataset. Pure Rust, no new heavy deps, almost certainly closes most of the gap. Add the dark-skin unit tests above so the improvement is visible.
- [ ] **Step 2 — only if step 1 isn't good enough.** Add `ort` and load one of the smaller models from samhaswon/skin_segmentation behind an optional feature flag (`features = ["ml"]`), keeping the classical path as the default so the binary stays small.

#### Guardrails
- [ ] Keep the per-Fitzpatrick-bucket eval as a checked-in test or bench so future changes can't silently regress fairness
- [ ] Update the doc-comment on `detect_skin` to honestly describe what the method does and its known limitations

## Slice: replay-based regression testing

### Target

Catch performance and cost regressions before they reach production. The motivating incident is the 26 Apr R2 Class A blowup (see `docs/current-slice.md`): a one-line change in `bc59e99` 10×'d the pruner's LIST rate against R2 and was only caught after deploy by reading Grafana graphs. The shape we want: capture real input traffic for a few minutes, replay the pipeline against a local backend, snapshot the resulting OTel counters, fail the test when any counter moves outside an expected band.

This is **not** deterministic simulation testing in the FoundationDB/TigerBeetle sense — no fake runtime, no concurrency exploration, no fault injection. Just: capture → replay → snapshot → diff.

What it catches:
* R2 op-count regressions (the 26 Apr incident)
* Queue-depth regressions (assert p99 of the depth gauge stays low)
* Throughput cliffs (events processed per simulated minute)
* Anything else expressible as an OTel metric

Out of scope:
* LLM cost regressions
* Concurrency bugs that would need a real DST framework
* Behaviours that only emerge under load longer than the fixture

### Tasks

#### Phase 1: replay infrastructure for pruner

* [ ] make the firehose source pluggable: `firehose::connect` currently returns a concrete `JetstreamReceiver`. Refactor the pipeline to accept any `Stream<Item = JetstreamEvent>` so a JSONL-backed source can drop in
* [ ] add a `capture` CLI that, for a given `--duration`:
    * records firehose events to `tests/fixtures/<name>/firehose.jsonl`
    * snapshots the live R2 store to a tarball at `tests/fixtures/<name>/store.tar`
    * records image HTTP GETs to `tests/fixtures/<name>/images/`
    * keep fixtures small enough to commit; if they grow past a few MB, move to git-lfs or an R2 fixtures bucket
* [ ] write a `replay_pruner` integration test that:
    * extracts the store tarball into a tempdir
    * opens the store via `file://` (existing `StoreArgs::open_store` path) wrapped in `R2MetricsWrapper` — the wrapper produces cost-equivalent counts against local disk
    * serves recorded image responses via [`wiremock`](https://github.com/LukeMathWalker/wiremock-rs)
    * drives the JSONL stream into the pluggable firehose source
    * runs until the stream ends, then `force_flush()`s the OTel meter and serialises all counters/gauges (the `InMemoryMetricExporter` pattern in `store_metrics.rs` already shows the shape)
* [ ] assert via a checked-in `expected-metrics.json` with explicit per-counter ranges (e.g. `r2.operations{operation="list"}: 60..120`). Prefer this over `insta`-style auto-blessing — clearer failure messages, no risk of someone blessing a 10× regression by reflex
* [ ] wire into `just test` so it runs in CI

#### Phase 2: extend to live-refine

* [ ] record OpenAI API responses keyed by request hash for the fixture window (wiremock or [`rvcr`](https://github.com/ChorusOne/rvcr))
* [ ] write `replay_live_refine` mirroring the pruner test, asserting both R2 ops and OpenAI request counts

#### Maintenance

* [ ] document how and when to refresh fixtures and the expected-metrics baseline — only when fixtures no longer represent production (e.g. firehose schema change, store schema change), not on every behaviour change
