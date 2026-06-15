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

### Tasks

> Ordering: do the per-crate inspection passes first (delete cruft so there's less to move), then the directory reorg, then the Justfile pass (so it lands on the final layout), then wrap-up.

#### Per-crate inspection passes

Each crate gets at least one full human pass: read all code, delete dead code, rework anything surprising, enforce the house rules along the way — `lib.rs` under 300 lines (extract modules if over), no `#[allow(dead_code)]`, no `unwrap`/`expect` in non-test code without a justified allow, and strip comment-rot (slice/phase/PR/task refs — see [[no-slice-phase-refs-in-code-comments]]). Note any non-obvious findings per crate.

* [ ] **Data/store layer — `skeet-store`.** The largest crate (~6.3k LOC); pay attention to module boundaries and the read/write paths. Run `just validate-storage` after.
* [ ] **Processing-pipeline binaries — `skeet-prune`, `skeet-refine`, `skeet-publish`.** The firehose → classify → score → publish chain.
* [ ] **Web services — `skeet-feed`, `skeet-appraise`.** The two HTTP-facing crates (banner/feed + auth-gated appraisals).
* [ ] **ML/detection libs — `face-detection`, `skin-detection`, `text-detection`.** Model loading/inference wrappers; confirm each model is still documented in `docs/`.
* [ ] **Shared/support libs — `shared`, `bluesky`, `web-support`, `build-support`, `test-support`, `eval`.** Cross-crate types and helpers; check `shared`'s types stay pure data (no policy methods).
* [ ] **Metrics exporters — `cloudflare-exporter`, `openai-exporter`.** Confirm both are still wired up and used; delete if obsolete.

#### Directory reorg

* [ ] **Move all crates under `crates/`.** Relocate every workspace member into `crates/`, then update: workspace `members` and `[workspace.dependencies]` path entries in the root `Cargo.toml`, each crate's own relative path deps, Dockerfile `COPY` paths (`Dockerfile.fly`, `Dockerfile.cluster`), and any just recipes / config that reference a crate path. `just clippy` + `just test-no-docker` must pass unchanged afterwards.
* [ ] **Move deploy artifacts under `deploy/`.** Relocate `fly.*.toml`, `Dockerfile.*`, and the root `*.env` files into `deploy/`, then fix the just recipes that reference them (`op run --env-file`, `fly deploy --config`, `docker build -f`). Verify with a dry-run / `fly config validate` where possible.

#### Justfile pass

* [ ] **Re-chunk and prune the just rules.** Review the root `Justfile` and `just/*.just`: split overgrown files (`store.just` ~125 lines, `cluster.just` ~170 lines) into more logical chunks, fold/relocate misplaced recipes, and delete any recipe that no longer makes sense (obsolete deploys, dead helpers). Keep imports consistent and `just --list` readable.

#### Wrap-up

* [ ] **Capture all changes and verify.** Ensure CLAUDE.md / docs references to moved paths are updated, then run `just clippy`, `just test-no-docker`, and `just mutants-on-diff` clean.
