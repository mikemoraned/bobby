# Current Slice: re-train refine model with a new eval snapshot

### Target

We've now amassed 1292 Image Appraisals so we'd probably benefit from capturing a new eval snapshot, and improved prompt. We've got bigger work planned later (see "try using embeddings for classification/scoring in refine") so we just want to focus this slice on allowing existing OpenAI model to be tuned based on a more up-to-date and complete set of image appraisals.

### Decisions / groundwork

- **The machinery already exists** — this is mostly an operational slice driving the bins built in earlier slices (`capture-appraisals`, `refine-eval`, `sample-costs`, `train`, `promote`) against the grown dataset. The checked-in config registries (`config/eval-splits.toml`, `config/eval-results.toml`, `config/refine.toml`) are the inputs/outputs; each step commits the registry it writes.
- **Stay on the existing OpenAI model.** Train keeps `gpt-4o` as both scorer and rewriter; the only thing changing is the prompt, tuned against a fresher/larger appraisal set. Embeddings and cheaper-model exploration stay out of scope (their own slices).
- **A fresh snapshot needs a fresh baseline.** `train`'s `pick_baseline` selects the best-F1 run carrying the production `model_version` **and the same `split_id`**. A newly captured snapshot has a new `split_id`, so no run qualifies until the deployed model is re-evaluated on it — hence the explicit re-baseline step before training.
- **`default` moves to the new snapshot** (capture's existing behaviour): the old phase-2–4 143-image split keeps its registry entry but loses the label, so `train`/`refine-eval` pick up the fresh snapshot automatically. This supersedes the "frozen 143-image split" the embeddings slice references — that slice should pin the old `split_id` explicitly rather than rely on `default`.

### Observations

##### 2026-07-03: re-baseline precision dropped on the new split — a calibration/distribution shift, not lost discrimination

Re-baselining the deployed model (`v2:34d8bec0`, gpt-4o, `decision_threshold = 0.5`) on the new `default` split (`3ad0a0b…`, 277 test images) gave **P=0.588, R=0.967, ROC-AUC=0.891** (TP/FP/TN/FN = 87/61/126/3), versus the historical phase-3 run of the *same model* on the old split (`a746519…`, 143 test): **P=0.800, R=0.870, ROC-AUC=0.897** (20/5/115/3).

Discrimination is unchanged (ROC-AUC ~0.89 both), but the whole score distribution sits higher relative to the 0.5 threshold: recall rose (positives score higher) **and** the false-positive rate on negatives jumped ~8× (4.2% → 32.6%). So this is a **calibration/distribution shift, not a discrimination loss** — the failure mode the "improving prune and refine quality" slice flagged as the binding ceiling. Likely driver: the appraisal queue is fed by the model's *own* candidates, so the appraised set is enriched with high-scoring hard negatives — making measured precision on it pessimistic-but-consistent (not evidence prod serves at 0.59 on the firehose). A plausible complementary read (owner): the incumbent, trained on far fewer labels, is simply **not good enough on the fuller/harder current set**.

Consequence for the gate: `train` will gate at "beat P=0.588 at R≥0.967−tolerance" — a valid *relative* bar on this split, but no longer coincident with the absolute 0.800 floor (that floor was set on the differently-composed old split, so the two precision numbers aren't directly comparable). Threshold recalibration for the new distribution is out of scope here — it belongs to the calibration work in the quality slice.

**Control (done — confounders ruled out):** the *current* production model re-run against the old `frozen-143` split gave **P=0.792, R=0.826, ROC-AUC=0.896, F1=0.809** — reproducing the historical phase-3 run (0.800/0.870/0.897/0.833) within gpt-4o's run-to-run noise (the recall gap is one image, 19 vs 20 of 23 positives). So gpt-4o API drift, eval-code changes, and price changes are all ruled out: the 0.588 on the new split is genuinely the distribution. **The incumbent still handles the old set but is no longer good enough on the fuller/harder current one — training is justified.**

**Gate shape (from `train/gate.rs`):** the inner loop selects the prompt with best F1 at threshold 0.5 (so it pushes precision up from the incumbent's 0.731 F1), but the acceptance gate is **relative** — it pins the candidate at the *baseline's* precision (0.588) and requires recall ≥ baseline − 0.01. "Accepted" therefore means "≥ the incumbent on this split", **not** "achieves 0.800"; on acceptance `train` auto-writes a `decision_threshold` tuned to 0.588. That is fine for this slice: the Target is to *tune the prompt against the fuller set*, and deploying the candidate as `train` writes it is not a regression (prod already sits at 0.588 on this set at threshold 0.5, and the appraisal set is hard-negative-enriched so real firehose precision is higher regardless). Re-asserting an absolute precision floor by deliberately re-choosing the threshold is a **calibration** concern owned by the "improving prune and refine quality" slice — explicitly **out of scope here**; we trust the relative gate this slice.

##### 2026-07-03: cost pre-flight (sample-costs, 11-image stratified sample, prices snapshot 2026-05-24)

Empirical per-image cost with the production prompt. **gpt-4o (the one we're training): avg $0.0029, min $0.0021, max $0.0030** — matches the ~$0.0028 implied by the full re-baseline/control eval runs, tight spread. `train`'s default $5 budget is ample: a 277-image full-test eval ≈ $0.80, leaving ~$4.20 for ~10 iterations (~145 images/iter). Other models (informational only — out of scope this slice): gpt-4.1-nano $0.0002, gpt-5-nano $0.0003, gpt-4.1-mini $0.0006, gpt-5-mini $0.0007, gpt-4o-mini $0.0037, gpt-5 $0.0050 avg/image.

##### 2026-07-03: pre-registered training protocol (recorded before any train run)

"Re-run only when the result is borderline" is optional stopping pointed at acceptance — it inflates the false-accept rate the same way peeking-until-significance inflates Type I error. To avoid it we fix the whole protocol up front, including which prompt we'd deploy (picking the best of N is selection bias unless the choice rule is pre-committed and isn't "the max"). Note two distinct noise sources: **process variance** (each `train` run yields a different prompt — the rewriter is stochastic) and **evaluation variance** (even a fixed prompt scores differently per pass). Running `train` N times and keeping the best conflates them and re-introduces selection bias; we neutralise that with a pre-committed non-max pick.

Protocol (`just train-replicated`):
- **N = 3** runs at seeds **42 / 43 / 44** (distinct per-iteration subsampling → independent candidates).
- **Baseline auto-selected** as the best-F1 eval of the *production* model on this split — by construction the re-baseline run (`019f282e…`), and stable across all three runs because candidates carry new model_versions and so can never be picked as baseline (no `--baseline-run-id` needed).
- Each run writes its candidate to a scratch copy of `refine.toml`; **`config/refine.toml` is untouched** during the protocol. All three append to `config/eval-results.toml`.
- **Decision:** adopt a retrained prompt **iff ≥2 of 3 pass the gate**; else the incumbent stands (snapshot reaffirmed it).
- **Which prompt (anti-selection):** among passing runs deploy the **median test recall** candidate (tie → the *lower*). All candidates are tuned to the same 0.588 precision, so recalls are comparable; median avoids deploying the luckiest draw.

##### 2026-07-04: N=3 replication outcome — adopt seed 42 (marginal win; calibration is the real lever)

All gpt-4o, gated against the re-baseline (`019f282e`, P=0.588 / R=0.967 / AUC 0.891 @thr 0.5):

| seed | verdict | P | R | F1 | ROC-AUC | thr | model_version |
|---|---|---|---|---|---|---|---|
| 42 | ACCEPT | 0.603 | 0.978 | 0.746 | 0.888 | 0.600 | `v2:ac1dde52` |
| 43 | ACCEPT | 0.578 | 0.989 | 0.730 | 0.872 | 0.650 | `v2:1835e655` |
| 44 | REJECT | 0.667 | 0.844 | 0.745 | 0.884 | 0.300 | `v2:4ff58310` |

**2 of 3 passed → adopt** (pre-registered threshold met). Median-of-passing recall (0.978, 0.989) is a tie → pre-committed *lower* → **seed 42** (`v2:ac1dde52`, thr 0.600); its scratch registry copied to `config/refine.toml`. (Seed 43 first crashed on an OpenAI client-init error before any scoring and was re-run — an infra failure, not a disliked-outcome re-roll, so protocol integrity holds.)

**Honest read:** the win is a small operating-point shift, not a discrimination gain. Seed 42 F1 0.746 vs baseline 0.731 (+0.015); ROC-AUC 0.888 ≈ baseline 0.891 (all three candidates at/below it). Seed 42 does Pareto-beat the incumbent on this split (higher P, R *and* F1 at its threshold), so adopting is not a regression and honors the pre-registration — we do **not** back out on "effect is small" grounds (that would be the post-hoc move the pre-registration exists to prevent). Seed 44 is instructive: it moved toward higher precision (0.667) at lower recall and was rejected, showing the gate — anchored to the incumbent's extreme-recall (0.967) operating point — structurally rewards recall-pushers over precision-improvers. The real ceiling remains calibration/discrimination, explicitly the "improving prune and refine quality" slice; this retrain refreshes the prompt against the fuller set without touching that lever.

### Tasks

* [x] **Capture the fresh snapshot.** Run `capture-appraisals` against the shared store to write a new `default` split from all current image appraisals (~1292) into `config/eval-splits.toml`. Sanity-check the train/test counts and per-band stratification look right; commit the registry.
* [x] **Re-baseline the deployed model on the new split.** Run `refine-eval` with the current production `config/refine.toml` against the new `default` split, appending a baseline run (new `split_id`) to `config/eval-results.toml` — this is what `train` gates against. Commit the log.
* [x] **Control: re-run the incumbent on the old split.** Re-run the current production model against the old `frozen-143` split (`refine-eval … frozen-143`) to isolate distribution shift from gpt-4o API / eval-code / price drift. Reproducing the historical ~0.800/0.870 confirms the re-baseline drop is the split (incumbent no longer good enough on the fuller set); diverging implicates a confounder to investigate before trusting either baseline. See the 2026-07-03 observation. **Result: reproduced (0.792/0.826) — confounders ruled out, training justified.**
* [x] **Cost pre-flight.** Run `sample-costs` over a small stratified sample to confirm `gpt-4o`'s empirical per-image cost under current prices before committing a training budget. **Result: gpt-4o ≈ $0.0029/image (tight); $5 budget ample. See the 2026-07-03 observation.**
* [x] **Train an improved prompt (N=3 pre-registered replication).** Run `just train-replicated` (`gpt-4o`, three seeds, gated against the auto-selected production-model eval on this split) per the pre-registered protocol in the 2026-07-03 observation. Do **not** re-roll to shop for an accept — that's optional stopping. Adopt iff ≥2 of 3 pass; deploy the median-recall passing candidate; else the incumbent stands. (Full variance-aware gating — mean/CI, re-baseline replication, in-loop overfitting — remains the later "improving prune and refine quality" slice; this is the minimal unbiased version.) **Result: 2/3 accepted → adopted seed 42 (`v2:ac1dde52`, thr 0.600); marginal win. See the 2026-07-04 observation.**
* [x] **Promote + deploy** (only if a candidate is accepted). Repoint the `production` label via the `promote` bin and do the k8s image cutover (per `docs/versioning.md` + `docs/remote-setup.md`) so the new model serves in prod. If rejected, record that the fresh snapshot reaffirmed the incumbent and skip the flip.
    * *Promote already done:* `train` wrote `production → v2:ac1dde52` into `config/refine.toml` (identical to what `promote set` does) — no separate `promote` step needed.
    * *Coordinated reader+writer cutover:* `config/refine.toml` is **baked into both** the `live-refine` image (the score **writer**) and the `skeet-publish` image (a score **reader** that filters `WHERE model_version ∈ registered set`). Rolling only `live-refine` would have it write `v2:ac1dde52` scores that an un-updated `skeet-publish` **discards** as unknown — the feed would stop getting fresh scores. So rebuild+roll **both**; roll `skeet-publish` first (widen the reader known-set) then `live-refine` (start writing the new version). Runbook: commit → `just push-skeet-publish` + `just push-live-refine` → `just cluster-deploy-skeet-publish` + `just cluster-deploy-live-refine` → verify `just cluster-status` / `just cluster-logs-live-refine`.
    * *Deployed (2026-07-04):* both `skeet-publish` and `live-refine` rolled to the new image; `v2:ac1dde52` confirmed serving as the Refiner version in Bobby Admin.
* [ ] **Verify + document.** `just clippy` + `just test-no-docker`. Record the snapshot `split_id`, baseline run-id, candidate run-id, and outcome (accepted+promoted, or rejected) so the comparison is reproducible.
