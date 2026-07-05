# Next Slices

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

## Slice: correct appraisal selection bias (rebalance sampling + low-band backfill)

### Target

Appraisals are drawn from the published feeds (the appraise homepage's feed dropdown — `recency-48h`, `quality-*`), which *are* the refine model's own top-ranked candidates. So the manual appraisal set skews to MediumHigh/High and feeds that skew back into training. The "re-train refine model with a new eval snapshot" slice already caught the symptom: on a fresh split, precision fell sharply at the deployed threshold while ROC-AUC held — discrimination intact, calibration/distribution the problem — because the queue enriches for hard negatives. The half that slice *didn't* name: the region the model **buries** (genuinely-good content it scores Low/MediumLow) is never queued at all, so the one signal that would reveal false negatives is structurally absent.

This slice starts correcting the label distribution *going forward* (steady-state rebalance) and closes the historical gap (a one-off low-band backfill), while recording enough provenance on each appraisal to interpret — and later reweight — the mix. It deliberately does *not* try to statistically salvage the existing high-biased appraisals in place; see the IPS decision below.

### Decisions / groundwork

- **Rebalance mode on the appraise homepage, default on, self-throttling.** Alongside the selected feed, when the feed has N unappraised skeets, also surface N (total) randomly-drawn unappraised skeets from the MediumLow/Low `Band`s. Tying the count to the existing backlog keeps appraisal demand bounded — it corrects the distribution without piling on. This score-independent draw is the load-bearing part: it's the only mechanism here that breaks the feed→appraise→train feedback loop.

- **Rebalance and backfill are one sampler at two tempos.** A single seedable "sample N unappraised skeets in bands {MediumLow, Low}, excluding already-appraised" capability on the store read port (`ScoredView`/`Scores`); the homepage consumes it throttled, and a dedicated `/backfill` endpoint consumes it unthrottled as a standalone queue. Both share the already-appraised exclusion set so they never double-serve. LanceDB has no native random sample; an in-memory shuffle of a band-filtered scan is fine at current volume — revisit if the scored table outgrows it. Seed the draw so it's reproducible in tests (matches the proptest habit).

- **Richer appraisal context than a bare enum.** Record on each appraisal the selection *mechanism* (`FeedRanked` / `Rebalanced` / `Backfilled` / `Unknown`) plus the **band-at-selection** and the **`model_version`** that assigned it. Band is model-version-relative (`NormalizedScore` + `decision_threshold`), so "Low" only means "Low per that version" — without the version the draw can't be reconstructed later. Name by mechanism, not effect, since the bias is derivable from the mechanism but not vice versa. This provenance *is* the propensity information, so it keeps reweighting/IPS on the table as a fallback without committing to it now. New context column(s) on `manual_skeet_appraisal_v1`, read covariantly (existing rows → `Unknown`).

- **Backfill over IPS, and adaptive rather than a blind quota.** Prefer collecting real MediumLow/Low labels to reweighting history: the buried region has ~zero/degenerate selection propensity, so IPS would be reweighting signal that was never collected — it can't recover exactly the region we care about. Don't pre-commit to matching the ~1400 existing appraisals; appraise a few hundred low-band first and read the buried-positive rate, which is itself the finding — low means the skew was mostly a calibration artefact (little to fix), high means a threshold move / retrain is warranted. `/backfill` only enqueues candidates; the labels still come from a human.

- **Balanced ≠ representative — evaluation stays on a separate hold-out (or is reweighted).** The rebalanced/backfilled set is deliberately even across bands, but the true pruned population is Low-dominated. That's right for *training* coverage and for measuring per-band true-positive rate (how we surface buried positives), but precision-at-threshold — the metric that regressed — is base-rate-sensitive. So the eval path (`refine-eval`, the frozen splits) must use a separately-drawn representative/random audit slice, or reweight to true band frequencies, rather than the rebalanced pool. Otherwise we fix the training distribution while still measuring on the wrong one — the same trap one layer up.

## Slice: try using embeddings for classification/scoring in refine

### Target

Refine currently routes every image through an LLM scorer — expensive, slow, and the prompt is the thing we're trying to optimise. Alternative worth measuring: embed each image once with a pre-trained vision model and learn a linear classifier on the embeddings. The embedding does the heavy lifting (visually/semantically similar → close in embedding space), so a linear SVM or logistic regression on top usually recovers a calibrated good/bad score that maps onto Low/MedLow/MedHigh/High bands. Inference drops from ~seconds and per-call cost to milliseconds and free; the prompt-optimisation problem disappears.

- **Deterministic — kills the variance problem.** A frozen embedding plus a seeded linear classifier produces a reproducible score: no temperature noise, no rewriter stochasticity, none of the 0.696–0.870 run-to-run recall spread that made the phase-4 gate hard to read. The re-run-N-times-and-take-a-confidence-interval machinery the LLM path needs just goes away.
- **Embed once, store forever.** Compute each image's embedding a single time and persist it (a new column/table in the lance store); re-scoring under a new classifier is then milliseconds and zero API calls. Contrast the LLM path, where re-scoring the ~34k stored images under a new model is exactly what CrashLoopBackOff'd live-refine (current-slice backfill incident) — here, retraining the classifier and re-scoring everything is trivial.
- **Embedding model matters more than the classifier.** [OpenCLIP](https://github.com/mlfoundations/open_clip) or [SigLIP](https://huggingface.co/docs/transformers/en/model_doc/siglip) for broad visual-semantic; [DinoV2](https://github.com/facebookresearch/dinov2) for pure-visual fine-grained. If the embedding can't see the distinction, nothing downstream recovers it.
- **Calibrated probabilities are the point, not a bonus.** A previous slice showed the cheap LLMs discriminated fine but were mis-calibrated — their scores didn't reach the 0.800-precision operating point without dumping recall. A learned head that emits calibrated 0–1 probabilities natively attacks exactly that binding constraint. The retrain slice re-confirmed this on fresh data: ROC-AUC held ~0.89 while the deployed model's precision at threshold 0.5 fell to 0.588 on the new snapshot — discrimination intact, calibration the ceiling.
- **Classifier choices.** Logistic regression via [`linfa-logistic`](https://crates.io/crates/linfa-logistic) gives calibrated 0–1 probabilities natively (preferred); linear SVM via [`linfa-svm`](https://crates.io/crates/linfa-svm) + Platt scaling is the alternative; kNN against curated prototypes is a no-training baseline; one-class SVM / SVDD if "bad" is too diffuse to label well.
- **Runtime in Rust.** Embeddings via [`ort`](https://crates.io/crates/ort) (same dep we'd add for the skin-detection slice) or [`candle`](https://github.com/huggingface/candle) for pure-Rust. CPU-only on hetzner (no GPU) — CLIP/SigLIP base is ~tens-of-ms/image, fine at current firehose volume and folds into the fixed monthly cluster cost.
- **Tradeoff + hybrid.** Gives up controllability over *which* aspect of similarity matters; if good/bad hinges on tone/intent the embedding can't see, the LLM scorer still wins. So treat it as the primary gate with the LLM kept for borderline cases — which is also the safe cost bet: if the classifier confidently decides ~80% of images and only the ~20% near the boundary escalate to the LLM, that's an ~80% cost cut without betting everything on full replacement.

### Phase 1: decisive offline experiment (cheap to falsify)

Retire the central risk in an afternoon before building anything, reusing the slice-16 `eval` crate end-to-end:

* [ ] Embed the ~1400 appraised images with 2–3 candidate models (e.g. SigLIP, OpenCLIP, DINOv2); cache embeddings to disk/store so every later step is instant
* [ ] Train `linfa-logistic` (cross-validated) on the new `default` split (`3ad0a0b…`, ~1400 appraisals — the label growth this experiment wanted has landed), labels from `Band::is_visible_in_feed()`. The old phase-2–4 split is pinned as the `frozen-143` label if you want the exact phase-4 comparison instead.
* [ ] Evaluate on the held-out test set and compare **recall-at-pinned-precision** against the LLM baseline re-measured on the *same* split — the deployed model is now `v2:ac1dde52`, and on the `default` split the prior incumbent sat at P≈0.588 / R≈0.967 (retrain slice re-baseline), **not** the stale phase-4 0.870 @ P=0.800.
* [ ] Caveat: on the new split the head is far better fed than the phase-4 ~88 positives, but "bad" is still the minority class — if logreg underperforms, try kNN/one-class before concluding the embedding can't see the distinction.

## Slice: speed-up pruner to increase coverage

### Target

As of 29th June, based on recent attempts to improve performance, it seems like the main bottleneck is cpu performance in the image filtering stage. This already maxes out the cores, so additional parallelism doesn't help. This bottleneck probably means we can't consume as much of the bluesky feed as we like. This slice is about trying out different optimisations that could help.

#### Establish Baseline

We don't know for sure what the upper limit is, so change the pruner so that it can run in different modes where it only enables subsets of the pipeline. So, for example, if we want to see what speed would be like if we didn't check any images, we can run it in a mode where it only enables the firehose (Ingest) and metadata stages. We'd have to make sure we don't accidentally save outcomes from this + make sure we don't pollute any metrics; it probably should still emit metrics, just under a different name e.g. `pruner-firehose,meta` (for enable stages).

We also may want to consider running this on laptop and in cluster, to assess impact of network speed, perhaps temporarily disabling the main pruner and other services so to avoid contention.

#### Try fail early / lazily evaluate based on specificity

We don't let an image through if it fails any the rejection reasons. We could perhaps save time by finding which of the checks has highest specificity (catches most items) and running that first, and running rest when these fail.

We can effectively have a fast path (lazy evaluate, stop early) and a slow path (evaluate all). We may want to keep both available so we can retest later.

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
