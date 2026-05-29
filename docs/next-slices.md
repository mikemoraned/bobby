# Next Slices

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
