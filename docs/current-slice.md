# Current Slice: re-train refine model with a new eval snapshot

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
