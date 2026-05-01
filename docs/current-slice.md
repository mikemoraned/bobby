# Current Slice: Slice 16 — make costs visible and reduce them

### Target

I'd like to end up with a monthly cost profile which is roughly the following, ordered by intended dominant costs:
1. prune + live-refine: fixed monthly cost of the hetzner cluster running them
2. live-refine: there will be a variable some number of image candidates each month, but I'd like a per-day upper-bound on spend on LLM calls, which turns into effectively a fixed cost per month
3. feed running on fly.io: small cost per call from blusky feed
4. admin/appraising: small ad-hoc cost as I appraise images on fly.io

However, what I actually have, as of 19th Apr is:
1. Significant R2 costs, coming from Class A and B operations which go above the free allowance; this is easily $100's per month if left unchecked
2. live-refine LLM costs: I've been manually topping this up by $5 a day, which easily get eaten-up; this may lessen once the effect of the more tight text-detection based pruning kicks in
3. prune + live-refine: hetzner cluster running code: €10 or approx £8.7 on hetzner cluster
4. feed + admin/appraising running on fly.io: $1 or approx £0.74 per month

### Observations

#### 24th Apr

The `skeet-feed` sends about 2.5K Class B operations. This kinda makes sense now in that there is a background job that refreshes once a minute. 

`skeet-prune` and `skeet-live-refine` seems to both do a *lot* of `get` and `get_range` requests (both send up to 30K per minute of each, for a period of about 4 minutes each). During this time other operations like `head`,`list` and `put` are tiny (10's per minute) I can sort-of understand why live-refine might need to do a lot of gets to get an image (though would be good if it's not lots of requests), however I don't see why pruner would have to.

#### 25th Apr

Tempo spike confirmed query plan data is available in traces. Two slow queries observed on every `list_unscored_image_ids_for_version` tick:

* `list_unscored:scored_ids` — 1.51s. Plan: `LanceRead` on `images_score_v2.lance`, projection `[image_id]`, 4 fragments, uses `ScalarIndexQuery` on `model_version_idx`. Slow despite the index.
* `list_all_image_ids_by_most_recent` — 2.04s. Plan: `LanceRead` on `images_v6.lance`, projection `[image_id, discovered_at]`, **66 fragments, no filter, no index** — full table scan every tick. This is almost certainly the dominant source of `get`/`get_range` R2 traffic from `skeet-live-refine`.

Checked the `compact` cron job to test the hypothesis that fragments were piling up. They are not — the cron is healthy (running every 10 min, completing in ~60–140s). The fragment count is **stuck at ~64 by design**: `skeet-store/src/compact.rs:50` sets `target_rows_per_fragment: 500` (low on purpose, because each row carries a ~2MB PNG blob and the lance default of 1M would OOM the compactor). Lance's planner only flags fragments with `physical_rows < 500` as candidates, so anything ≥500 rows is left alone forever. Each cron run merges only the small stragglers (e.g. "365 rows across 2 fragments" + "4 rows across 4 fragments") and lands back at ~64 fragments (mean=433, p50=500, p99=2022). Implications:

* Compaction is **not the lever** — pushing `target_rows_per_fragment` up would re-introduce OOM risk.
* The full scan in `list_all_image_ids_by_most_recent` will remain expensive as long as it scans every fragment.
* Our `RECOMMEND: compact: 64 small fragments (>10 threshold)` health line is misleading — it uses lance's default smallness threshold, not our chosen 500. (Worth fixing separately.)
* This makes the case stronger for both *Idea: reduce cost of polling in live-refine* (version-snapshot early-abort skips the scan when nothing changed) and *Idea: make `list_all_image_ids_by_most_recent` use `discovered_at_idx`* (let lance scan the index instead of every fragment).

#### 1st May

##### merge_insert verification (1st May, deployed ~15:35)

Two data files:
* `metrics_dumps/r2 operations rate — images_score_v2.lance (_versions)-data-as-joinbyfield-2026-05-01 21_27_10.csv` — all services (`r2_operations_total{table="images_score_v2.lance", kind="_versions"}`)
* `metrics_dumps/r2 operations rate — images_score_v2.lance (_versions)-data-as-joinbyfield-2026-05-01 21_41_44.csv` — live-refine only (same query + `service_name="skeet-live-refine"`)

**Result: inconclusive — spikes return later in the post-deploy window.**

Data files:
* Short post-deploy window (15:35–18:31, 177 min): 0 spikes from live-refine — looked promising initially.
* Extended window (08:42–20:42) from `metrics_dumps/r2 operations rate — images_score_v2.lance (_versions)-data-as-joinbyfield-2026-05-01 21_44_23.csv`:

| metric | live-refine only — before (08:42–15:34, 413 min) | live-refine only — after (15:35–20:42, 308 min) |
|---|---|---|
| spike events (>10 ops/s) | 2 (08:46, 14:03) | 3 (18:44, 19:20, 20:02) |
| spike event rate | 0.29 /hr | 0.58 /hr |
| peak `get`+`get_range` | 181 ops/s | 183 ops/s |

Spike intensity is unchanged (~180 ops/s, 3 min each), and the rate if anything increased. The first 3h post-deploy happened to be quiet (no large batches to score); once scoring activity resumed the spikes came back at the same scale.

Adding two more metrics for the same window (`metrics_dumps/live_refine images scored per minute-data-2026-05-01 21_48_09.csv`, `metrics_dumps/live_refine R2 ops per scored image-data-2026-05-01 21_49_17.csv`) reveals the pattern clearly:

| condition | scored / min | R2 ops / scored image | `_versions` ops/s |
|---|---|---|---|
| normal scoring (61 minutes) | 0.5–1.3 | ~50–140 | < 1 |
| spike scoring (7 minutes) | 0.3–1.0 | **11,000–32,000** | ~180 |

Scoring happens regularly across 68 minutes in the window, but spikes occur in only 7 of them (10%). The scoring *rate* is similar in both cases — the only thing that changes is R2 ops/image jumps 100–160×, entirely attributable to `_versions` reads. The first minute of each spike always shows 0 images scored (R2 ops/image = inf), meaning the burst precedes scoring output, consistent with the read phase of the scoring cycle (`list_unscored_image_ids_for_version` + `merge_insert`'s table scan) driving the cost, not the image fetch.

What distinguishes the 10% of cycles that spike from the 90% that don't is not visible from these metrics alone — candidates are batch size, gap since last activity, or accumulated manifests making the `LIST _versions/` walk longer. But the 160× R2/image ratio shows the spike is a qualitatively different operating mode, not just a larger version of normal.

**Why spikes persist despite merge_insert:** unknown from these metrics alone. What we *do* know (from the manifest count below) is that the floor for any single Strong-mode manifest resolve on `images_score_v2` is ~18 R2 ops (17 LIST pages + 1 manifest GET). So whatever causes some cycles to issue many resolves, each resolve is ~18× more expensive than it would be after pruning. We have not traced whether the multi-resolve loop comes from `list_unscored:scored_ids`, the `merge_insert` internals, or somewhere else — that's a separate investigation.

**What this means for the remaining fixes:**

* **(2) Prune** is still needed: old manifests accumulate regardless, making the `LIST _versions/` walk progressively more expensive — likely the reason some cycles spike and others don't (manifest count growing over uptime).
* **(3) Drop Strong mode** is the primary lever for spike intensity: a TTL on manifest resolution would remove the per-read `LIST _versions/` + `GET` from both `list_unscored_image_ids_for_version` and the `merge_insert` scan, collapsing the 160× R2/image spike back toward the normal ~100 ops/image baseline.

##### Manifest count measurement (1st May, via `just count-versions`)

Added a small CLI (`skeet-store/src/bin/count-versions.rs`) that uses the AWS SDK to LIST `<table>.lance/_versions/` for every table and report manifest count plus the number of R2 LIST API pages required (R2 returns max 1000 keys per page).

```
table                             manifests list_pages       oldest_h       newest_h
--------------------------------------------------------------------------------------
images_v6                             12187         13          337.1            0.1
images_score_v2                       16461         17          337.1            0.1
manual_skeet_appraisal_v1                80          1          333.0            6.5
manual_image_appraisal_v1               745          1          335.5            4.9
validate_v1                              35          1          608.1            5.4
```

**Findings:**
* `images_score_v2` has **16,461 manifests requiring 17 R2 LIST pages** per `LIST _versions/` call. `images_v6` is similar (12,187 / 13 pages). The other three tables fit in 1 page.
* The oldest manifest is 337 hours (≈14 days) old. There is currently no pruning at all — manifests accumulate from the moment a table is created.
* This makes the pagination hypothesis no longer a hypothesis: every Strong-mode refresh on `images_score_v2` does ~17 LIST page fetches + 1 manifest GET = ~18 R2 ops minimum, before any data read. Normal scoring at ~50–140 R2 ops/image is consistent with a small number of these refreshes per cycle plus image fetch.
* This does not yet explain why only 10% of cycles spike to ~180 ops/s — that requires something doing many refreshes in a tight loop during those cycles. But it confirms that the per-refresh cost is structurally high *because of unpruned manifests*, and pruning would lower the floor for every Strong-mode operation regardless of why the spike-loop happens.

**Implication for fix order:** Prune (fix 2) is now the obvious first step — measurable baseline (16k+ manifests, 17 pages), measurable target (≤1 page after prune), and it lowers the floor for fix (3) when we get there. Setting `older_than: 1h` on the cron prune action would shrink the active manifest count to ≈10–20 (one cron run's worth at most cadences), bringing the LIST cost back to a single page.

#### 30th Apr

##### Watermark verification: traces (29th Apr) + R2 ops (28th Apr)

Verifying the `since`/watermark optimisation in *Idea: reduce cost of polling in live-refine* (introduced in commit `eb4e0be`, deployed 2026-04-28 17:36–17:40).

**Trace evidence (29th Apr, via `just trace-summary skeet-live-refine list_all_image_ids_by_most_recent`):** all 10 sampled `list_all_image_ids_by_most_recent` spans show the planner picking `ScalarIndexQuery` on `discovered_at_idx`, with the watermark pushed down as `discovered_at >= TimestampMicrosecond(...)`. The `fragments: 67` field in the plan is the table total — actual `read_fragment` calls visible in the child `DatasetRecordBatchStream` spans are typically 2–5 per query, confirming index pruning works. Span wall time is still ~1.3–1.9s, but that cost lives in the index lookup itself (the sibling `list_unscored:scored_ids` query against `model_version_idx` shows similar 1.2–2.6s) — not in fragment scans.

**R2 op evidence (28th Apr, comparing 208 min before deploy to 220 min after):**

| metric | before | after | Δ |
|---|---|---|---|
| mean `get` / min | 905 | 629 | -30% |
| **median `get` / min** | **275** | **47** | **-83%** |
| mean `get_range` / min | 685 | 579 | -16% |
| spike count (>10K/min) | 7 min / 2 events | 6 min / 2 events | ~same |
| peak ops / min | 48K | 45K | ~same |

The watermark did exactly what it was meant to do *on the idle path*: median-minute `get` collapsed 6× as ticks where `images` table version + watermark say "nothing new" no longer fire the listing scan. Background `get` total (≤1K/min minutes) dropped 43K → 9K, a 78% cut.

Spikes are unchanged because they're a different workload — image-fetch (`get_originals_by_ids` pulling ~4MB PNG blobs), which only runs when unscored candidates exist. The watermark suppresses the polling scan, not the scoring work.

Spikes are 22–24K `get` + 22–24K `get_range` simultaneously (~1:1 ratio), 1.3–1.7h apart. Diagnosed below.

##### Spike-cost diagnosis (30th Apr, follow-up)

Pulled `r2.bytes / r2.operations` per minute and per-`(table, operation)` ops from Grafana. Two clear observations:

1. **Spike `get_range` averages ~1.0 KiB/op** (consistent across all 16 spike minutes either side of the deploy). Idle minutes typically 4–8 KiB/op. The earlier framing was "many tiny page reads" vs "few large blob reads" — the data lands firmly on **many tiny page reads**. *(Caveat: our wrapper records bytes for `get_range` and `put` only — `get` bytes aren't captured because we'd need to consume the response. From the ops counters we know spike-minute `get` ops match `get_range` ~1:1, but their byte size is unknown.)*

2. **The spike is on `images_score_v2.lance`, not `images_v6.lance`.** Per-table breakdown of the 19:19–19:20 spike (40K ops/min total): `images_score_v2.lance / get`: 20,234, `get_range`: 19,700; `images_v6.lance / get`: 53, `get_range`: 1. Confirmed across all 8 top spike minutes in the 7-hour window — every one is dominated by `images_score_v2.lance` at 20–24K `get` + 20–24K `get_range`, with `images_v6.lance` contributing <1%. That points at `batch_upsert_scores` (the upsert-merge has to read existing scores) or its read-side companion `cached_scores` rebuilding.

3. **Window totals (before vs after deploy) reinforce both points:**

   | table | before (208 min) | after (220 min) | Δ |
   |---|---|---|---|
   | `images_v6.lance` | 43,413 | 7,108 | **-84%** |
   | `images_score_v2.lance` | 292,477 | 263,621 | -10% |

   The watermark cut `images_v6.lance` ops 6× as designed. But `images_score_v2.lance` was already ~7× more expensive than `images_v6.lance` *before* the watermark, and is ~37× more expensive after — so the elephant in the room was always the scores table; the watermark just exposed it more starkly.

Both observations re-frame the spike-cost problem and feed two new ideas (below): adding a `kind` sub-label to disambiguate within a table, and investigating scores-table read amplification.

##### Spike-cost localisation by `kind` (30th Apr, after `kind` label deployed)

`kind` label deployed; pulled per-`(table, kind, operation)` rates from Grafana for the 5 highest spike minutes (`metrics_dumps/live_refine operations total by table, kind & operation-…2026-04-30 12_24_58.csv`). Pattern is consistent — every spike is **~99%+ `images_score_v2.lance / _versions / {get, get_range}`**, split roughly 1:1:

| time (UTC) | total ops/min | `_versions` (% of total) | `_indices` | `data` | other |
|---|---|---|---|---|---|
| 01:45 | 43,629 | 43,294 (99%) | 63 | 25 | 247 |
| 11:11 | 41,900 | 41,701 (100%) | 7 | 17 | 175 |
| 11:10 | 41,900 | 41,701 (100%) | 7 | 17 | 175 |
| 11:09 | 41,850 | 41,693 (100%) | 13 | 11 | 133 |
| 10:03 | 41,818 | 41,590 (99%) | 57 | 13 | 157 |

Implications for *Idea: reduce scores-table read amplification on upsert*:
* It is **not index amplification** (`_indices` is rounding error) and **not per-fragment data reads** (`data` is rounding error).
* It is **manifest churn**: the scores table is at `_versions/N.manifest` files being read repeatedly. `get`/`get_range` ~1:1 fits "list-or-head a version, then range-read manifest bytes."
* Direction shifts toward: how `batch_upsert_scores` (and its `cached_scores` reader counterpart) interact with manifests. Either the writer is producing many version bumps per batch (each a new manifest), or the reader is re-resolving versions per row. Either way the next step is to instrument or read the lance write path to confirm where the manifest reads originate.

> **Forward-pointer (1st May):** Fix (1) below addressed the first horn (collapsed N+1 → 1 commit per batch), and was deployed — it did *not* measurably reduce spikes. The 1st-May manifest count (16k+ unpruned manifests on `images_score_v2`, 17 LIST pages per resolve) shows the dominant cost is the per-resolve floor, not how many commits the writer produces. See 1st May Observations.

#### 26th Apr

##### R2 Class A + `list` operation usage regression

Summary of issue / investigation:

Observed in Grafana: large increase in Class A usage. Drilling per-CLI:
* `pruner`: `list` operations spiked from a baseline of ~16/min to a steady ~250/min, with occasional bursts to ~350/min
* `pruner`: `image` stage pipeline queue depth jumped from a typical 0 to regular spikes up to 60

Deployed version: `1128a57c69acb6c1c7bbd70209eafce1f9983545`. Previously deployed: `cb840de2c9fb1146f8130d4794316cb7d05f67a7`. Commits in between (newest first):

```
1128a57 Add `table` label to r2.operations / r2.bytes via per-call path parsing
bc59e99 Add lance.table.fragments gauge via SkeetStore::fragment_counts()
a282968 Add r2.bytes counter to R2MetricsWrapper
c69a4bf, c61ddb9, 2480e5a   docs-only edits to current-slice.md
```

Of the three code commits, only `bc59e99` adds new R2 traffic. `a282968` and `1128a57` only label/observe existing calls.

**Root cause: `SkeetStore::fragment_counts()` is mis-described as cheap.** Added in `bc59e99` (`skeet-store/src/lib.rs`), the doc comment reads *"Cheap: reads only the manifest"* but the implementation calls `lancedb::Table::stats()` once per table × 5 tables. Reading lancedb 0.27 source (`lancedb-0.27.2/src/table.rs:2563`), `stats()` actually:

1. `count_rows(None)`
2. `list_indices()` → for each index, `index_statistics()` → `collect_regular_indices_statistics` opens `LanceIndexStore::from_dataset_for_existing` and calls `scalar::fetch_index_details` per index (LIST/HEAD per index uuid directory)
3. `calculate_data_stats()` → for **every fragment, every column**: `FileFragment::storage_stats` → `open_readers` (file header GETs and HEAD/LIST per fragment dir)
4. Per-fragment `physical_rows()`

For `images_v6.lance` alone: ~64 fragments × multiple columns of opens, plus per-index stats for each scalar index (`discovered_at_idx`, `image_id_idx`). `images_score_v2.lance` adds `model_version_idx`, plus three more tables with their own indices.

This call is invoked from two new sites:

* **skeet-prune** (`save_stage.rs:19-24`): inside the `rx.recv()` loop, gated by `is_time_to_log()`. But `is_time_to_log` only flips back to false when `maybe_log()` runs, and that only fires from `record_post()` (the `Post` arm). Until the next `Post`, every `Classified`/`Rejected` arrival re-enters the `if`, and `fragment_counts().await` runs again. Worse, the `.await` blocks the receiver, so the `image` MPSC backs up — explaining the queue-depth spike.
* **skeet-refine** (`live_refine.rs:237-239`): once per tick (default `--interval-secs 60`). Smaller impact than pruner but still adds 5 stats calls/min.

Default `--status-interval-secs 30` for pruner, default `--interval-secs 60` for live-refine. Even the floor (one stats call/30s × 5 tables for pruner, plus 1/60s × 5 for live-refine) is plausibly the ~234 LIST/min delta observed.

Short-term workaround tasks:

* [x] roll `pruner` back to the previous image tag — the per-hash tagging from `6a5010f` predates `cb840de`, so the image already exists in ghcr.io. No rebuild needed:

  ```sh
  just cluster-rollback-pruner cb840de
  ```

  Re-renders `pruner-deployment.yaml` through `envsubst` with the supplied tag and `kubectl apply`s — keeps `image:` and `OTEL_RESOURCE_ATTRIBUTES.service.version` in sync. **Don't use `kubectl set image` for this**: it only updates the image field, leaving `service.version` pointing at the previous tag, so metrics in Grafana keep reporting the old version even though a different binary is running.
* [x] verify in Grafana that `r2_operations_total{cli="pruner",operation="list"}` drops back to the ~16/min baseline within a few minutes
* [x] verify the `image` stage `pipeline.depth` gauge drops back to ~0
* [-] (optional) same rollback for `live-refine` if its contribution is still material once pruner is rolled back: `just cluster-rollback-live-refine cb840de`
* [-] roll forward again once the long-term fix below is built — `just cluster-rollback-pruner <new-short-hash>` (or re-run `just cluster-deploy-pruner` from the new HEAD)

Long-term fixes:

Strategy: move fragment-count reporting out of the hot path entirely, and into the `compact` cron job — fragment count is a compaction concern. Cron cadence is 10 min (`compact-cronjob.yaml:8`), which is the right resolution for a "compaction drift" gauge. Also fix the underlying cheapness assumption so any future hot-path use is safe.

* [x] in `skeet-store/src/lib.rs`, swap `table.stats().await?` → `table.as_native()?.count_fragments().await?` (lancedb 0.27 `NativeTable`). That just reads the cached `Dataset` manifest. Updated the (previously false) "Cheap: reads only the manifest" comment to be accurate. The optimisation matters even at 10-min cadence — `stats()` triggers index-stats reads + per-fragment per-column `open_readers`.
* [x] in `skeet-store/src/bin/compact.rs`, construct `StoreMetrics` via `opentelemetry::global::meter("lance")`, call `store.fragment_counts().await?`, and `record_fragment_counts(&counts)` after the post-compact `storage_health` block. Emit **unconditionally** — even when `needs_action()` returns false and the binary exits early at line 38. Otherwise the gauge only updates after compactions actually run, which hides the drift signal we want to see *between* runs. The existing `TracingGuard` (held as `_guard` in `main`) flushes the `MetricsGuard` provider on drop before exit.
* [x] in `skeet-prune/src/save_stage.rs`, remove the `is_time_to_log()` / `fragment_counts` block (lines 20–24) entirely.
* [x] in `skeet-prune/src/status.rs`, remove the `store_metrics` and `fragment_counts` fields, the `is_time_to_log()` and `update_fragment_counts()` methods, the `record_fragment_counts` call in `log_summary`, and the `StoreMetrics` import.
* [x] in `skeet-refine/src/bin/live_refine.rs`, remove the `let store_metrics = ...` line and the `if let Ok(counts) = store.fragment_counts().await { ... }` block at the end of the loop, plus the `StoreMetrics` import.
* [x] redeploy `compact`, `pruner`, `live-refine` with the fix and seen no regressions

Probes after the long-term fix is deployed:

* `count_over_time(lance_table_fragments[15m])` should be ~5 (one emission per table per cron run, every 10 min)
* `r2_operations_total{cli="pruner",operation="list"}` should stay at the ~16/min baseline
* `r2_operations_total{cli="compact",operation="list"}` may rise slightly (one cheap manifest read per table per run) but should be negligible vs the cron's normal compaction traffic
* `image` stage `pipeline.depth` gauge should stay at ~0 in steady state

### Tasks

#### Get visibility on R2 usage

I've registered for grafana cloud, so can use that instead of honeycomb, which may be easier to use. 

Docs:
* traces: https://grafana.com/docs/grafana-cloud/send-data/traces/
* metrics: https://grafana.com/docs/grafana-cloud/send-data/metrics/#ways-to-connect-your-data-to-grafana-cloud

Details for OLTP:
OTEL_EXPORTER_OTLP_PROTOCOL="http/protobuf"
OTEL_EXPORTER_OTLP_ENDPOINT=op://Dev/bobby-grafanacloud-oltp-endpoint/password
OTEL_EXPORTER_OTLP_HEADERS=op://Dev/bobby-grafanacloud-oltp-headers/password

* [x] upgrade lancedb from 0.26 to 0.27 (lance-io 2.0.0 → 3.0.0)
    * do this as a standalone task before the wrapper work
    * check for breaking changes in lancedb 0.27 CHANGELOG
* [x] migrate to grafana cloud as the endpoint to which traces are sent
    * `shared::tracing` (`shared/src/tracing.rs`) already uses standard OTLP via env vars (`OTEL_EXPORTER_OTLP_ENDPOINT`); currently points at Honeycomb for all CLIs (Hetzner + fly.io)
    * [x] create a small test CLI (`skeet-store/src/bin/otel-test.rs`) that sends sample trace spans using `shared::tracing::init_with_file`, then exits
        * add a `just` target to run it via `op run` with the Grafana Cloud env vars
        * still use standard opentelemetry apis; no grafana-specific code
    * [x] if that works, update env files for Hetzner and fly.io deployments — should be env var changes only, no code changes
* [x] implement a `WrappingObjectStore` to count R2 operations per CLI
    * this should log a metric for every particular S3 API operation used
    * ideally this should easily map to R2 Class A or Class B actions
    * the outcome I want is a graph over time of operations per-cli so I can see which cli is using the most operations, and how those split out per operation for a particular cli
    * **Approach: `lance_io::object_store::WrappingObjectStore` trait**
        * trait has one method: `fn wrap(&self, store_prefix: &str, original: Arc<dyn ObjectStore>) -> Arc<dyn ObjectStore>`
        * decorates the built-in S3 store — lance still handles credentials, multipart, commit semantics
        * the wrapper delegates every call to the inner store but emits OTel metrics (counters by operation type + CLI name)
        * S3 operations to track: GET/HEAD → R2 Class B; PUT/DELETE/LIST → R2 Class A
    * **Plumbing into lancedb**
        * pass wrapper via `ObjectStoreParams { object_store_wrapper: Some(Arc::new(wrapper)), .. }`
        * thread into table operations via `lance_read_params()` / `lance_write_params()` on `OpenTableBuilder` etc.
        * note: `ReadParams` uses field `store_options`, `WriteParams` uses field `store_params` (asymmetric naming)
        * all table operations go through `SkeetStore` methods, so plumbing is contained
    * **Dependency: `lance-io`**
        * lancedb 0.26 → lance-io =2.0.0; lancedb 0.27 → lance-io =3.0.0
        * upgrade lancedb first (task above), then add lance-io =3.0.0

#### Get visibility on overall pipeline performance and content stats 

* [x] `skeet-prune`: emit OTel metrics (same Grafana Cloud endpoint as R2 visibility) at the same cadence as the periodic status log line. Emit raw cumulative counts as counters (let Grafana compute rates). Example log output these are derived from:
```
2026-04-24T20:01:04.482091Z  INFO skeet_prune::status: skeets: 10443 (0.8/s) | images: 10391 | saved: 24 (0.2%) | rejected: 12695 (BlockedByMetadata: 2349 [17%], FaceNotInAcceptedZone: 153 [1%], FaceTooLarge: 30 [0%], FaceTooSmall: 1017 [7%], TooFewFrontalFaces: 7440 [54%], TooLittleFaceSkin: 382 [3%], TooManyFaces: 1289 [9%], TooMuchSkinOutsideFace: 538 [4%], TooMuchText: 529 [4%]) | categories: Face: 10253 [81%] (sole: 9817 [77%]), Text: 529 [4%] (sole: 93 [1%]), Metadata: 2349 [19%] (sole: 2349 [19%])
2026-04-24T20:01:04.482139Z  INFO skeet_prune::status: pipeline | throughput: firehose=10461 (0.8/s), meta=10444 (0.8/s), image=8094 (0.6/s) | depth: firehose=16, meta=0, image=0
```
    * **Performance metrics** (from the `pipeline` log line):
        * `skeet_prune.pipeline.throughput` — counter, label `stage` ∈ {`firehose`, `meta`, `image`}
        * `skeet_prune.pipeline.depth` — gauge, label `stage` ∈ {`firehose`, `meta`, `image`}
    * **Content metrics** (from the content log line):
        * `skeet_prune.skeets.total` — counter (cumulative skeets seen)
        * `skeet_prune.images.total` — counter (cumulative images seen)
        * `skeet_prune.saved.total` — counter (cumulative images saved)
        * `skeet_prune.rejected.total` — counter, label `reason` ∈ {`BlockedByMetadata`, `FaceNotInAcceptedZone`, `FaceTooLarge`, `FaceTooSmall`, `TooFewFrontalFaces`, `TooLittleFaceSkin`, `TooManyFaces`, `TooMuchSkinOutsideFace`, `TooMuchText`}
        * `skeet_prune.categories.total` — counter, label `category` ∈ {`Face`, `Text`, `Metadata`}
        * `skeet_prune.categories.sole.total` — counter, label `category` ∈ {`Face`, `Text`, `Metadata`} (images where that category was the sole detection)
* [x] `skeet-live-refine`: emit OTel metrics at the end of each poll tick (after batch-saving scores). Cumulative counters; let Grafana compute rates.
    * **Throughput metrics:**
        * `skeet_live_refine.images.unscored` — counter (cumulative images found unscored at the start of each tick)
        * `skeet_live_refine.images.scored` — counter (cumulative images successfully scored)
        * `skeet_live_refine.images.errors` — counter, label `reason` ∈ {`ImageEncoding`, `Completion`, `ParseScore`}
    * **Score distribution:**
        * `skeet_live_refine.scores` — OTel `Histogram<f64>`, one observation per scored image, explicit bucket boundaries `[0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]` (gives 10 buckets covering 0.0–1.0)
    * **Approach:** add a `metrics.rs` module to `skeet-refine` (same pattern as `skeet-prune`); wire `PruneMetrics` → `LiveRefineMetrics` at the bottom of the `loop` body in `live_refine.rs` once per tick

#### Bring k8s image tagging closer to best-practice by not using `latest` and instead the git hash

* [x] add `envsubst` (part of GNU `gettext`) to the `prerequisites` target — already present on dev machine via Homebrew but should be explicit
* [x] in `just/container.just`, add a second `-t` to each `push-*` target tagging the image with `{{ GIT_HASH }}` (keep `:latest` so existing references don't break)
* [x] in each k8s manifest (`pruner-deployment.yaml`, `live-refine-deployment.yaml`, `compact-cronjob.yaml`):
    * replace the hardcoded tag in the `image:` field with `${IMAGE_TAG}` (e.g. `image: ghcr.io/mikemoraned/bobby/pruner:${IMAGE_TAG}`)
    * add `imagePullPolicy: IfNotPresent` — correct behaviour for immutable tags (no unnecessary re-pulls on pod restart)
* [x] in `just/cluster.just`, change each `cluster-deploy-*` target to pipe through `envsubst` before applying: `IMAGE_TAG={{ GIT_HASH }} envsubst < infra/k8s/<name>.yaml | kubectl apply -f -` — all non-image changes in the YAML are still applied, and the tag is pinned to the exact pushed commit

#### Add git hash to traces and metrics

* [x] ensure that git-hash, as software version, is added to all traces and metrics as metadata
    * do this in an OTEL-standard way e.g. anything that corresponds to a `version` or similar
    * the intent is to allow all metrics and traces to be filtered in Grafana Cloud by what has been deployed, so that I know that a metric came from a particular version of the software

#### Use Grafana API to extract Trace data

I am using Grafana Cloud and there will be traces that correspond to things like `list_unscored_image_ids_for_version` which are objects of possible optimisation. There should also be lancedb query plans attached to these spans as tracing events (not span attributes).

**How plan data gets into traces:** `execute_query` in `lancedb_utils.rs` calls `explain_plan(true)` and logs the result via `debug!`/`warn!`. These are tracing *events* (child items of a span), forwarded to Tempo by `tracing_opentelemetry`. Key implications:
* You can't filter by plan content in TraceQL search (TraceQL filters on span attributes, not event fields)
* You *can* see the plan once you fetch the full trace — events appear inside the span
* Only plans on queries exceeding 100ms (`SLOW_QUERY_THRESHOLD`) are logged at `warn` level. Faster queries use `debug!`, and whether those reach Tempo depends on the RUST_LOG / default filter level. If the default is `info`, only slow query plans will be present.

The goal is to ground optimisation decisions in real data (actual query plans, column projections, scan behaviour) rather than just textual analysis of code.

* [x] quick spike to check the data is available at all
    * [x] create a Grafana Cloud service account token with traces:read scope (if not already done)
    * [x] store the Tempo endpoint URL and token in 1Password
        * `bobby-grafanacloud-tempo-url` contains endpoint url in the `password` field
        * `bobby-grafanacloud-tempo-token` contains token in the `password` field, and the username in the username field
    * [x] use curl + jq to:
        1. search for traces: `GET /api/search` with TraceQL `{resource.service.name="skeet-live-refine"}`
        2. fetch one full trace by ID: `GET /api/traces/{traceID}`
        3. confirm that query plan text appears in span events
    * [x] example saved in `spans-example.json`
* [x] if spike successful:
    * [x] add Tempo read credentials (endpoint + token) to `bobby-grafana-otel.env` as 1Password refs
    * [x] write a cli, in `skeet-store` which can:
        1. Find a sample (e.g. 10) of traces within a time window which contain a call or calls to any `SkeetStore` method
        2. Extract call hierarchy and any plans that were logged as events on the span
        3. Summarise this textually in a way which will be understandable by a person and a reasonable LLM
            * a particular focus should be on things which affect the cost of a query e.g. which columns were loaded and how many rows were loaded
        * implemented in `skeet-store/src/tempo.rs` + `src/bin/trace-summary.rs`; run via `just trace-summary`
    * [x] delete example saved in `spans-example.json` as shouldn't be needed anymore
    * [x] remove any other Justfile rules or files we created for the spike e.g. `tempo-search`
    * [x] add fixture-based tests to guard against parsing regressions (e.g. `traceID` vs `traceId`)
        * add a `capture-trace-fixtures` just target (uses `bobby-grafana-otel.env`) that saves a real search response and one full trace response to `skeet-store/tests/fixtures/`; run once and commit
        * inline unit tests in `tempo.rs` using `include_str!` over the fixtures: verify deserialization succeeds and key fields (e.g. `trace_id`) are non-empty
        * inline unit tests in `trace_analysis.rs` using hardcoded plan strings (no fixture needed): cover full-scan detection, indexed-query detection, and slow-query event extraction
    * [ ] if needed, we can update `SkeetStore` to attach (via log/span entry) more useful data about query cost related metadata
        * [ ] replace bespoke plan string parsing in `trace_analysis.rs` with flat typed attributes emitted at log time in `lancedb_utils.rs` — parse the raw `explain_plan` once into a `QueryPlan` struct, then pass each field as a named arg to `warn!` (e.g. `plan.table`, `plan.num_fragments`, `plan.full_scan`, `plan.full_filter`, `plan.index`) so they land as native OTel event attributes. Removes `extract_field` / `plan_summary` string hacks in `trace_analysis.rs` (consumer just reads attributes, no JSON parse).
            * **Status:** code change landed; `QueryPlan` parser added in `skeet-store/src/query_plan.rs`, producer (`lancedb_utils.rs`) emits flat fields, consumer (`trace_analysis.rs`) reads them. **Not yet validated end-to-end against real Tempo data** — committed fixture (`tempo_trace_response.json`) still has the legacy single `plan` string. After redeploy, re-run `just capture-trace-fixtures` and re-enable the `#[ignore]`'d `slow_query_extracted_from_real_fixture` test in `trace_analysis.rs` to confirm the wire format matches.
            * **Why flat attributes, not a JSON blob:** OTel semconv for databases (`db.system`, `db.statement`, `db.operation`) has no convention for query plans, but the universal convention for everything else is flat KV — that's what Tempo/TraceQL can filter on. JSON-in-an-attribute is opaque to TraceQL, escapes badly in the Tempo UI, and forces the consumer to parse. Flat attributes make queries like `event.plan.full_scan=true` (a regression detector) possible.
            * **Scope:** our plans are simple — one `LanceRead` plus optional `ScalarIndexQuery` — so flattening doesn't lose useful tree structure. If joins ever appear, revisit (could nest under `plan.read.*`, `plan.index.*`).
            * **Note:** lancedb's `explain_plan` only returns a free-form `String` (datafusion's `Display` output, no `Serialize` impl). Parsing has to happen on our side regardless; this just moves it from consumer to producer and emits structured fields instead of a string.

##### Add more detailed metrics

* [x] add a bytes counter to `R2MetricsWrapper` alongside the existing `r2.operations` counter
    * new metric `r2.bytes` — `Counter<u64>` with the same label set (`cli`, `store_prefix`, `operation`, `r2_class`)
    * record `range.end - range.start` for `get_range`; sum of ranges for `get_ranges`; payload size for `put`/`put_opts`
    * for `get`/`get_opts`/`head` either skip (no bytes signal without consuming the response) or record the resulting `ObjectMeta::size` if cheap to obtain
    * goal: distinguish "many tiny page-header range reads" (unindexed scans needing per-page I/O) from "few large blob reads" (multi-MB image fetches) — a bytes/op ratio in Grafana makes this immediate
* [x] emit a per-table fragment-count gauge so we can see if compaction is keeping up
    * new metric `lance.table.fragments` — `Gauge<u64>` with label `table` ∈ {`images`, `scores`, `skeet_appraisal`, `image_appraisal`, `validate`}
    * source the value from `Table::stats()` (already a lightweight manifest read; called at startup in `open.rs`)
    * emit once per `live-refine` tick (and same cadence in `skeet-prune` / wherever cheap), not per query
    * goal: detect compaction drift directly — if `images` fragments climb past the 25 Apr baseline of 66, the cron job is not keeping up and the cost of every full scan grows with it
* [x] add a `table` label to `r2.operations` (and `r2.bytes`) by parsing the per-call `location: &Path`
    * today `WrappingObjectStore::wrap()` is invoked once at connect time, so `store_prefix` is effectively a constant per-CLI (`s3$hom-bobby`) — useless for breaking R2 traffic down by table
    * the per-call `location` argument carries the actual path, e.g. `encrypted-store/images_v6.lance/data/xxx.lance` or `encrypted-store/images_score_v2.lance/_versions/123.manifest`
    * extract the first path segment ending in `.lance` (e.g. `images_v6.lance`) and emit it as a `table` label on every `record()` call; fall back to `unknown` if no segment matches
    * goal: in Grafana, group `r2_operations_total` by `(table, operation)` for a given `cli` to confirm — concretely — that `images_v6.lance` is the dominant burst source and which operations dominate within it
    * note: the `store_prefix` label can stay (still useful as a sanity check that the wrapper is wired) but `table` becomes the primary grouping dimension
* [x] add a per-op latency histogram to `R2MetricsWrapper` alongside `r2.operations` and `r2.bytes`
    * new metric `r2.duration` — `Histogram<f64>` (seconds), same labels as the others (`cli`, `store_prefix`, `table`, `kind`, `operation`, `r2_class`)
    * record wall-clock around the inner-store delegate call in each wrapper method (`get`, `get_opts`, `get_range`, `get_ranges`, `head`, `list*`, `put*`, `delete*`)
    * goal: distinguish "spike is many requests" from "spike is slow requests"; gives a baseline for any future infra change (e.g. evaluating SSE-C contribution, or comparing prefixes/regions). Confirmed (1st May, by reading `object_store` 0.12.5 + `lance-io` 4.0.0 source) that SSE-C does not defeat lance's or object_store's range-coalescing layers, so SSE-C is not a likely cost driver — but per-op latency is still useful as a general debugging tool independent of the SSE-C question.

##### Get visibility of R2 metrics from Cloudflare in my Grafana metrics

Intent:

Pull R2 operation and storage metrics directly from Cloudflare's GraphQL Analytics API and push them into the same Grafana Cloud tenant we already use, so Cloudflare's ground truth sits alongside our in-app `r2.operations` / `r2.bytes` counters on the same dashboards. The motivation is twofold: (a) validate that the in-app `R2MetricsWrapper` numbers match Cloudflare's billing-aligned counts, and (b) any gap between the two reveals R2 traffic that isn't going through the wrapper (e.g. paths we missed, side-channels). Cost correlation with deploys then falls out for free, since `service.version` (git hash) is already on every other metric in the tenant.

Design decisions:
* New crate `cloudflare-exporter` with a single `sync` CLI
* Source: Cloudflare GraphQL Analytics API, datasets `r2OperationsAdaptiveGroups` (Class A/B counts dimensioned by `actionType`, `bucketName`) and `r2StorageAdaptiveGroups` (`payloadSize`, `objectCount`). 31-day retention, ~5-min ingestion lag.
* Sink: OTLP push via the existing `shared::tracing` setup — delta-temporality sums for operation counts, gauges for storage. No new auth path, no Prometheus scrape endpoint to host.
* Schedule: k8s CronJob, once per minute. Default window queries `[now − 6min, now − 5min]` so we always read settled data; `--from`/`--to` flags override for ad-hoc runs.
* Label parity with `r2_metrics.rs` (`bucket`, plus `action_type` / equivalent of our `operation`) so a Grafana panel can show Cloudflare-truth vs in-app counters with `join` on the same dimensions.
* Grafana Cloud Mimir's 2h out-of-order window comfortably absorbs once-a-minute writes; no tenant config change needed.
* Caveat: Cloudflare's API does **not** expose path-prefix (`data/` vs `_versions/` vs `_indices/`) — that detail still only exists in our in-app `kind` label. Cloudflare gives bucket-level totals only.

Tasks:

* [x] provision a Cloudflare API token scoped `Account Analytics: Read`; store it in 1Password as `bobby-cloudflare-analytics-token` (and the account tag as `bobby-cloudflare-account-tag`)
* [x] scaffold a new `cloudflare-exporter` crate in the workspace; add the corresponding `cloudflare-exporter.env` (1Password refs for the API token, account tag, and the existing Grafana Cloud OTLP env vars)
* [x] implement `cloudflare.rs`: typed GraphQL client for `r2OperationsAdaptiveGroups` (group by `actionType`, `bucketName`) and `r2StorageAdaptiveGroups`. One integration test behind `op run` that hits the real API and asserts the response shape (kept small; gated like other live-API tests)
* [x] implement `otlp.rs`: emit operation counts as OTel `Counter<u64>` (one observation per `(bucket, action_type)` per window) and storage as `Gauge<u64>`. Reuse `shared::tracing` for provider setup so `service.version` flows through automatically
* [x] wire `sync` CLI (clap): `--from`, `--to` overrides, default to `[now − 6min, now − 5min]`. Capture invocations in the Justfile (`just cloudflare-sync`), running through `op run --env-file cloudflare-exporter.env`
* [x] add a k8s CronJob manifest in `infra/k8s/` (once per minute, same image-tag pattern as the rest — `${IMAGE_TAG}` + `envsubst`); add a `cluster-deploy-cloudflare-exporter` just target
* [ ] verify in Grafana that a `cloudflare_r2_operations_total` (or whatever metric name we settle on) series appears with the expected labels and the per-minute count is non-zero
* [ ] build a Grafana panel that overlays Cloudflare-truth vs the in-app `r2_operations_total` for the same `bucket`, so a divergence is visually obvious — the comparison that actually validates this work

#### Idea: Remove inline compaction in favour of the cron job

The `compact` cron job already runs every 10 minutes against all tables. The `compact_every_n_writes` mechanism duplicates this inline, blocking the save path and generating large GET/GET_RANGE bursts against R2 during each run.

* [x] remove `compact_every_n_writes` from `StoreArgs` and `SkeetStore` entirely
* [x] remove the `compact_if_needed` call sites in `lib.rs` and `scores.rs`
* [x] remove the `writes_since_compact` counter from `SkeetStore`

#### Idea: Batch image fetches in live-refine

`live_refine.rs` fetches images one at a time via `get_by_id` inside a loop, generating O(N) separate R2 queries each returning a full `StoredImage` (~4MB: original + annotated PNG blobs). Live-refine only needs the original image for scoring.

* [x] replace the per-image `get_by_id` loop (`live_refine.rs:78-97`) with a single `store.get_by_ids(&batch_ids)` call before dispatching the scoring batch
* [x] make `annotated_image` optional in `StoredImage` (e.g. `Option<DynamicImage>`), and add a fetch mode or separate query path that skips the `annotated_image` column — so callers like live-refine that don't need it don't pay the R2 cost

#### Idea: Only update feed cache on version change

Ultimately it'd be good for this to be more of a push-on-change approach, where a central cache is updated when something has changed about scoring or similar. However, for now, I think we can have a different approach i.e.

* [x] update `SkeetStore` to have a `version_snapshot` method which returns a `HashSet<Version>` where
    * `Version` is a struct with a `name` and `tag`
        * `name` is the name of the underlying table
        * `value` is an opaque identifier capturing the version of the table
    * this `value` should be a `String` to keep non-coupled to the underlying implementation, but which should be derived from the `version` of each underlying lancedb table
* [x] update the `skeet-feed` cache so that it still runs once a minute but functions as follows when it wants to test if cache needs updated:
    1. fetch `version_snapshot`
    2. filter `HashSet<Version>` down to only the `name`'s it depends to invalidate the cache:
        * so, for example, it is only a change in appraisals or image scores that should effect the cache; changes to images or skeets does not affect it
    3. (assuming this `HashSet<Version>` has been previously saved on the cache) compare those against what has just been found
    4. if they are different then proceed as now in invalidating and updating the cache
* [x] we can also remove the staleness check as this method should mean we don't need it anymore
* [x] all of the above should be done in a failing-test-first way as we are introducing more complexity here

The outcome of this should be that we only incur the cost of updating the in-memory cache when something has changed.

#### Idea: reduce cost of polling in live-refine

Every poll tick, `live-refine` runs `list_unscored_image_ids_for_version`, which scans both `images` and `scores` tables — reading LanceDB's arrow fragment files from R2 to filter IDs. This generates `get`/`get_range` calls even though no image blobs are fetched. If unscored IDs are found, `get_originals_by_ids` then fetches the actual image data (~4MB per image). The ID scans are paid every tick regardless of whether anything has changed.

`SkeetStore` will expose a `version_snapshot` as part of "Idea: Only update feed cache on version change". We can use the `images` table version from that snapshot as a cheap early-abort: if the table version hasn't changed since the last tick, no new images were committed and the expensive scan can be skipped entirely. `table.version()` is already used in `cached_scores()` and is a lightweight manifest read — not a scan.

We'll do this in stages:
* [x] (observation) emit an OTel gauge from `SkeetStore` reporting the observed `version` for each table (label `table` ∈ {`images`, `scores`, ...}), updated on each access. This lets us see in Grafana how often the `images` table version actually changes per minute — if it changes every tick, the early-abort optimization gives no benefit and we should reconsider before building it.
* [x] implement a dashboard/panel in Grafana that shows how version changes over time for each table
    * **Result (27th Apr overnight):** `images_v6` has gaps of up to 40 minutes with no version change, more commonly ~6 minutes, and is frequently ≥2 minutes between changes. This confirms the early-abort is worth building — many ticks fire with no new images, each paying a full 64-fragment scan unnecessarily.
* [x] (prerequisite) "Idea: Only update feed cache on version change" is implemented, giving us `version_snapshot` on `SkeetStore`
* within `skeet-refine`, separate polling from dispatch:
    * [x] extract the poll-and-fetch step from `live_refine.rs` into a `PollingBatchSource` struct in `skeet-refine/src/polling.rs`:
        * holds `store: Arc<SkeetStore>`, `model_version: ModelVersion`, and `last_images_version: Option<u64>` as state between ticks
        * exposes an async `fetch(&mut self) -> Result<Batch, StoreError>` method; `Batch` is constructed via `From<Vec<StoredOriginal>>`
        * on each call: fetch `table_versions()`, extract the `images` table version, and return an empty `Batch` immediately if unchanged since last call
        * if changed: run `list_unscored_image_ids_for_version` + `get_originals_by_ids`, update `last_images_version`, and return the candidates
    * [x] update `live_refine.rs` main loop to call `source.fetch()` instead of doing the query inline; dispatch is a single `dispatch(&mut candidates, ...)` call

##### Variation: hold `last_discovered_at` and push it down as a filter

The version-snapshot above is binary (changed / not changed). When the table *has* changed, we still scan every fragment looking for unscored ids. A finer variation: also remember the maximum `discovered_at` that `PollingBatchSource` has seen successfully scored, and pass it back into the store as a `WHERE discovered_at > last_discovered_at` filter on the next tick. That filter is exactly the predicate the `discovered_at_idx` BTree (created in `open.rs:90-98`) can satisfy, and the projection `[image_id, discovered_at]` is covered by the index — so when a tick does run, it should pay a BTree range read instead of a 64-fragment scan.

Naming/type notes (cross-checked with `skeet-refine/src/polling.rs` + `skeet-store/src/types.rs`):
* the existing struct is `PollingBatchSource` (not `PollingImageSource`); state is held there
* the timestamp newtype in this codebase is `DiscoveredAt` (wraps `DateTime<Utc>`); use that for the cutoff
* `Batch` currently exposes only `ids` + `images` — it needs to carry per-candidate `DiscoveredAt` plus per-id completion bookkeeping internally, so the live-refine loop reports completions back through the batch object itself

Edge case (decided): if scoring fails (e.g. `Completion`/`ParseScore` errors), the image stays unscored but its `discovered_at` is in the past — a strictly-monotonic cutoff would never retry it. We'll go with **option (a): only advance the watermark up to but not past the oldest unscored image in the batch**, leaving stragglers in-window. (Considered (b): keep the cutoff but run a periodic full-reconciliation pass — rejected as more complex for no extra correctness.)

Watermark rule (computed inside `Batch` at commit time):
* if every batch member was marked completed → watermark = `max(discovered_at)` of all members (we've fully processed them)
* if any batch member was not marked completed → watermark = `min(discovered_at)` of the not-completed members (advancing past those would lose them)

The store-side filter must be **inclusive (`discovered_at >= since`)** so the oldest-not-completed member is re-fetched on the next tick. (Already-scored members caught by the same filter are then weeded out by `list_unscored_image_ids_for_version`'s scored-id join — i.e. inclusive `>=` is safe even when the watermark sits on a completed boundary.)

* [x] add a `since: Option<DiscoveredAt>` parameter to `list_unscored_image_ids_for_version` (threading through to `list_all_image_ids_by_most_recent`) — when `Some`, push down a `discovered_at >= <ts>` filter on the `images` table query; when `None`, behave as today. TDD: add a store-level test that adds rows with two timestamps and asserts the `since` form returns only the newer subset (and includes the boundary row).
* [x] extend `Batch` (in `skeet-refine/src/polling.rs`) with private `discovered_at_by_id: HashMap<ImageId, DiscoveredAt>` (sourced from `StoredOriginal::summary::discovered_at` in `From<Vec<StoredOriginal>>`) and `completed: HashSet<ImageId>` plus a public `mark_completed(&id)` method. The live-refine loop calls `batch.mark_completed(&id)` for each successfully scored candidate during dispatch.
* [x] extend `PollingBatchSource` with `last_discovered_at: Option<DiscoveredAt>` state and a `commit(&mut self, batch: Batch)` method that consumes the batch, computes the watermark as above, and advances `last_discovered_at` (monotonically — never goes backwards). `fetch()` passes `self.last_discovered_at.clone()` as the `since` arg. Tests: (1) cold start with `None` returns full scan; (2) after `commit` of a fully-completed batch, next `fetch` skips already-scored items via the watermark; (3) after `commit` of a batch with stragglers, the oldest straggler is the watermark and re-appears on the next tick; (4) `commit` is monotonic — earlier watermarks don't roll back.
* [x] in `live_refine.rs`, drive the loop as: `let mut batch = source.fetch().await?;` → dispatch, calling `batch.mark_completed(&id)` for each successful scoring → `store.batch_upsert_scores(...)` → `source.commit(batch)`. Errors leave that id un-marked, so the watermark won't advance past it.
* [x] cold-start / restart behaviour: in-memory state means a fresh pod takes one full scan to bootstrap `last_discovered_at` — acceptable, no persistence needed.
* [x] verify in a real trace that when `since` is set, lance picks a `ScalarIndexQuery` on `discovered_at_idx` rather than a full `LanceRead` over the 64 fragments, and that R2 op counts per tick drop accordingly. **Verified 30th Apr — see Observations.** Trace plans show `ScalarIndexQuery` on `discovered_at_idx` on every sampled span (only 2–5 actual `read_fragment` calls per query, not 67). R2 ops: median `get`/min dropped 275 → 47 (-83%) on idle ticks. Spikes unchanged because they're image-fetch, not polling-scan — separate optimisation thread.

#### Idea: tie R2 metrics to current trace (exemplars)

The R2 metrics emitted by `R2MetricsWrapper` are not currently linked to any trace. Ideally each `counter.add(...)` would carry an OTel exemplar with the originating `trace_id`/`span_id`, so a spike in R2 ops in Grafana could be clicked through to the exact `SkeetStore` method span that caused it. Rust OTel SDK 0.31 attaches exemplars automatically from `Context::current()` — no API changes needed at the call site.

**Blocker: context propagation through lancedb/datafusion.** `tokio::spawn` does not carry tracing context into spawned tasks, and lancedb/datafusion spawn their own tasks for query execution. By the time the wrapper's `record()` runs, `Context::current()` is empty.

**Per-call wrapper workaround (writes only).** `write_options()` in `lib.rs:64` is called per-call inside an `#[instrument]`'d method, so `Context::current()` is correct at that point. We could capture it into a per-call `ContextualR2Wrapper`, then re-attach inside `record()`. Works cleanly for writes — but writes are <1% of our R2 cost.

**Read path: no per-query injection in lancedb 0.27.** Verified by reading the lancedb source:
* `QueryExecutionOptions` only exposes `max_batch_length` and `timeout` (`query.rs:582`)
* `ExecutableQuery` trait has no read-params hook (`query.rs:621`)
* `OpenTableBuilder.lance_read_params()` is the only `ReadParams` injection point (`table.rs:164`) — set once at table-open time
* Workaround would be re-opening the table per query (one extra `list_indices` + manifest GET per call) — likely not worth it

**Upstream context:** two open lancedb issues exist around `WrappingObjectStore` ergonomics — both about hoisting the wrapper to *connection* level, not per-query:
* [lancedb#3072](https://github.com/lancedb/lancedb/issues/3072) — Allow custom object store at connect time (open, quiet)
* [lancedb#3106](https://github.com/lancedb/lancedb/issues/3106) — Pluggable caching layer; maintainer endorses `WrappingObjectStore` as the right hook and supports connection-level inheritance
* No issues exist for per-query injection, OTel context propagation, or observability through the data path. We'd be the first to ask.

**Decision:** deferred. The pragmatic alternative — using the existing `store_prefix` (table name) label plus time-window correlation in Grafana, combined with the trace-summary tool — is good enough to ground cost-reduction work. Revisit if exemplar correlation becomes a recurring need, in which case file an upstream issue for per-query `object_store_wrapper` first.

#### Idea: add a `kind` sub-label to R2 metrics

Today `table_from_path` (`r2_metrics.rs:233`) extracts only the first `.lance` segment. Reads to `images_score_v2.lance/data/...`, `images_score_v2.lance/_indices/...`, `images_score_v2.lance/_versions/...`, and manifest files all roll up to the same `table` value. With the 30th-Apr finding that spikes hit `images_score_v2.lance` at ~20K `get`+`get_range`/min, we can't currently tell whether that's data-fragment reads, index-uuid lookups, or manifest churn — each points at a different fix.

* [x] add a `kind` label to `r2.operations` and `r2.bytes`, derived from the path segment immediately after the `<table>.lance/` directory: `data` / `_indices` / `_versions` / `_transactions` / `manifest` (top-level `.manifest` files) / `other`. Inline unit tests covering each path shape.
* [x] re-pull the per-`(table, kind, operation)` breakdown for a spike minute on `images_score_v2.lance` to localise the cost source within the table; record the result in Observations. **Result: ~99% of every spike is `_versions/{get,get_range}` — see Observations.**

#### Idea: reduce scores-table read amplification on upsert

The 30th-Apr per-table breakdown shows spike-minute R2 cost lives almost entirely on `images_score_v2.lance` (>99% of the 40K ops/min spike), not on the image-data table. Image-fetch batching is already done; that is *not* where the cost is. Per-`kind` follow-up confirmed **~99% of every spike is `_versions/{get,get_range}`** — manifest reads, not index lookups, not data fragments. See Observations 30th Apr.

Diagnosis (originally from lancedb 0.27 source review; items 2 and 3 confirmed by direct R2 measurement on 1st May — see Observations):

1. **Write side: N+1 commits per batch.** `batch_upsert_scores` (`scores.rs:54-96`) does `delete()` in a loop (one per row in the batch) followed by a single `add()`. In lance, every `delete()` and every `add()` is its own commit and writes a fresh `_versions/N.manifest` (confirmed in `lancedb-0.27.2/src/table/delete.rs:24-35` — even a delete with predicate `"false"` increments the version). A batch of N rows → N+1 manifests. *Fix (1) below collapses this to 1 commit per batch — verified correct by version-delta test, but did not measurably reduce spikes (see 1st May observations); the dominant cost turned out to be unpruned manifests making each Strong-mode refresh expensive, not the per-batch commit count.*
2. **Read side: every read resolves the latest manifest.** We open the DB with `read_consistency_interval(Duration::ZERO)` (`open.rs:28`), which puts the wrapper in lancedb's *Strong* mode. In Strong mode every `Table::version()` and every read calls `refresh_latest` → `LIST _versions/` + manifest GET. The 1:1 `get`/`get_range` ratio in our spikes is exactly that pattern (R2 LIST shows up as a `get_range`-style op). **Confirmed 1st May:** the 17-page LIST per resolve on `images_score_v2.lance/_versions/` is the concrete cost.
3. **No version cleanup.** `compact.rs:60-95` runs `OptimizeAction::Compact` + `Index` only, never `OptimizeAction::Prune` / `cleanup_old_versions`. Old manifests accumulate forever, so every `LIST _versions/` walks more keys over time. **Confirmed 1st May:** 16,461 manifests on `images_score_v2`, oldest 14 days — quantified via `just count-versions`.

Three fixes, ranked. They compose — none substitutes for another.

* [x] **(1) Replace the delete-loop in `batch_upsert_scores` with `merge_insert`** — collapses N+1 commits to 1 per batch. The lancedb `Table::update` doc explicitly recommends this pattern over per-row loops. Approximate shape:
    ```rust
    self.scores_table
        .merge_insert(&["image_id"])
        .when_matched_update_all(None)
        .when_not_matched_insert_all()
        .execute(Box::new(reader_over(batch)))
        .await?;
    ```
    Build one Arrow `RecordBatch` covering all rows, wrap in a `RecordBatchReader`, run a single `merge_insert`. Drop the `delete()`+`add()` pair. `merge_insert` retries on conflict by default — keep that. TDD: extend `store_tests::batch_upsert_scores_*` tests to assert the table version increments by exactly 1 per call regardless of batch size.

    **Post-deploy result (1st May, deployed ~15:35 — see Observations for data files):** the fix is *correct* (single commit per batch verified by version-delta test) but *insufficient on its own*. The expected R2 reductions did not materialise:
    * Spike intensity unchanged (~180 ops/s, 3 min each); spike-event rate if anything increased (0.29/hr → 0.58/hr) once scoring activity resumed post-deploy.
    * Peak ops/s on `images_score_v2.lance` essentially identical (181 → 183).
    * `_versions` ops/s during spikes still dominate at ~99%+ of the spike traffic — same shape as before.

    Why the prediction was wrong: either typical batch sizes were already small (so N+1 ≈ 1 and the writer-side saving is negligible), or the spikes were never write-driven in the first place. The 1st-May manifest measurement (16k+ manifests, 17 LIST pages) shows the per-resolve floor is structurally high regardless of how many resolves happen — pointing at fix (2) as the next lever.
* [ ] **(2) Add `OptimizeAction::Prune` to the compact cron** — without it, `_versions/` grows unbounded. Two options:
    * swap each `OptimizeAction::Compact` + `Index` pair for a single `OptimizeAction::All` (which is `compact_files` + `cleanup_old_versions(7d)` + `optimize_indices`), or
    * add an explicit third `OptimizeAction::Prune { older_than: Some(Duration::from_secs(3600)), delete_unverified: false, error_if_tagged_old_versions: false }` step.
    Prefer the explicit form so we can tune `older_than` (1h is plenty given a 10-min cron — 7d is overkill and lets weeks of manifests accumulate between deploys). Verify via Grafana: post-deploy, `LIST` ops on `_versions/` for `cli=skeet-live-refine` should plateau rather than drift up.
* [ ] **(3) Drop `read_consistency_interval(Duration::ZERO)`** — Strong mode is the wrong default for a system that does batch writes. Move to a small TTL (e.g. `Duration::from_secs(5)`) so reads can serve from lance's in-memory dataset cache between manifest resolves. Implications for `cached_scores` (which uses `version()` as the invalidation signal): with eventual consistency, `version()` may report a slightly-stale value. Either:
    * accept up-to-5s staleness on the score cache (probably fine — the feed already polls on a coarser cadence), or
    * replace `version()`-based invalidation with a time-based TTL on the cache itself, decoupling it from manifest resolution entirely.
    This one is the most behaviour-changing and should land *after* (1) — once batches are single commits, the version-bump-per-batch frequency drops by ~Nx and Strong mode hurts a lot less, so the urgency is lower. Still worth doing for read-cost reduction outside of spikes.

