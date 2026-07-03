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

**Control (in progress):** re-run the *current* production model against the old `frozen-143` split (pinned a durable label in `config/eval-splits.toml`) to isolate distribution shift from gpt-4o API / eval-code / price drift. Reproducing ~0.800/0.870 confirms the drop is the split; diverging implicates a confounder.

### Tasks

* [x] **Capture the fresh snapshot.** Run `capture-appraisals` against the shared store to write a new `default` split from all current image appraisals (~1292) into `config/eval-splits.toml`. Sanity-check the train/test counts and per-band stratification look right; commit the registry.
* [x] **Re-baseline the deployed model on the new split.** Run `refine-eval` with the current production `config/refine.toml` against the new `default` split, appending a baseline run (new `split_id`) to `config/eval-results.toml` — this is what `train` gates against. Commit the log.
* [ ] **Control: re-run the incumbent on the old split.** Re-run the current production model against the old `frozen-143` split (`refine-eval … frozen-143`) to isolate distribution shift from gpt-4o API / eval-code / price drift. Reproducing the historical ~0.800/0.870 confirms the re-baseline drop is the split (incumbent no longer good enough on the fuller set); diverging implicates a confounder to investigate before trusting either baseline. See the 2026-07-03 observation.
* [ ] **Cost pre-flight.** Run `sample-costs` over a small stratified sample to confirm `gpt-4o`'s empirical per-image cost under current prices before committing a training budget.
* [ ] **Train an improved prompt.** Run `train` (`gpt-4o`, gated against the re-baseline run). On acceptance it writes the new prompt/model + `decision_threshold` to `config/refine.toml` and appends the run to the log; on rejection the incumbent stands. A single run is noisy — if the candidate lands borderline against the gate, re-run before trusting it (full variance-aware gating is the later "improving prune and refine quality" slice).
* [ ] **Promote + deploy** (only if a candidate is accepted). Repoint the `production` label via the `promote` bin and do the k8s image cutover (per `docs/versioning.md` + `docs/remote-setup.md`) so the new model serves in prod. If rejected, record that the fresh snapshot reaffirmed the incumbent and skip the flip.
* [ ] **Verify + document.** `just clippy` + `just test-no-docker`. Record the snapshot `split_id`, baseline run-id, candidate run-id, and outcome (accepted+promoted, or rejected) so the comparison is reproducible.
