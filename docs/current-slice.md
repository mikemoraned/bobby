# Current Slice: Slice 12 — Optimisations of pruning, refining, and feeding

### Smells to be investigated / addressed

1. looking at cpu and network usage in hetzner:
    * we can see it going up and down:
        * cpu goes from 100% to 200% over a period of about 30 secs
        * network bandwidth goes from 0 to 80Mbps over similar time-scale
    * this feels a bit like a bottleneck being encountered periodically like a buffer being filled and meeting a limit
    * we also log every 30 seconds, so it's possible this is somehow blocking things (as it sits at end of pipeline)
2. looking at traces for operations like `get_by_id` these take about 10 seconds and make multiple `read_fragment` calls in `lance::io::exec::filtered_read`
    * this kinda looks like a table scan; couldn't this be an index lookup instead?
    * also, could we add a dump of the plan to the trace (in `plan_scan`) so we can see what it is doing?
    * I see similar traces for things like `exists`
3. the read of the feed itself takes several seconds
    * this could be caused by the same or similar problems as 2 i.e. long scans

### Optimisation ideas

* live-refine: when we find a new set of images to score:
    * we can dispatch multiple calls in parallel to openai as we're largely waiting on them to respond (it's i/o bound)
    * once we have some scores, we can batch-save them to lancedb (lancedb recommends batch-saving to reduce fragmentation)

### Benchmarking

* we should be able to measure the maximum possible speed the pruner can run by taking the jetstream stage only and running that on it's own
    * we should be able to run a minimal cli instance and associated k8s deployment which just runs this step and summarises speeds
    * probably should just run for 5 minutes and collate some statistics before dumping them out at the end

### Tasks

#### Smells (investigate first — #2 likely explains #3 and may contribute to #1)

##### Hypothesis A: Scalar index not being used by `get_by_id` / `exists` — DISPROVED

**Background:** `get_by_id` and `exists` (in `skeet-store/src/lib.rs`, lines ~212 and ~226) use `.only_if(format!("image_id = '{image_id}'"))` with `.limit(1)`. A scalar index on `image_id` is created at startup via `Index::Auto` (line ~82). However, traces show `get_by_id` taking ~10 seconds with multiple `read_fragment` calls in `lance::io::exec::filtered_read`, which looks like a full table scan rather than an index lookup.

**How to prove/disprove:**
* [x] Add `explain_plan(true)` logging to `get_by_id` and `exists` queries
    * lancedb 0.26.2 has `ExecutableQuery::explain_plan(&self, verbose: bool) -> Result<String>` — available on `Query`
    * Both `.explain_plan()` and `.execute()` take `&self`, so call both on the same built query object
    * Log at `debug!` level (visible with `RUST_LOG=skeet_store=debug`) to avoid noise in production
    * **What to look for:** plan should show `ScalarIndexExec` if index is used; if it shows `FilterExec` over a full `LanceScan`, the index is not being engaged
    * If index isn't used: possible causes are (a) `Index::Auto` doesn't create a BTree for `Utf8` columns, (b) the index exists but the query planner doesn't select it, (c) the index is stale (see below)
* [x] Also log `table.index_stats("image_id")` at startup for both `images_table` and `scores_table`
    * lancedb 0.26.2 has `Table::index_stats(index_name) -> Result<Option<IndexStatistics>>` returning `num_indexed_rows`, `num_unindexed_rows`, `index_type`
    * **Critical lancedb behaviour:** new data added after index creation is NOT covered by the index. Queries still return correct results, but the unindexed portion requires a flat scan. Only `optimize(OptimizeAction::All)` (or `OptimizeAction::Index`) updates indices to cover new data.
    * **What to look for:** `num_unindexed_rows > 0` means the index is stale and queries are partly scanning unindexed fragments. If `num_unindexed_rows` is close to `num_rows`, the index is effectively useless.

**Result (2026-04-06):** Disproved. All `get_by_id` query plans show `ScalarIndexQuery: query=[image_id = ...]@image_id_idx` — the scalar index is being used, not a full table scan. No `exists` calls appeared in the log (live-refine doesn't call `exists`; that's the pruner path), but the same index is used so it's expected to behave identically.

**Observations:**
* `index_stats("image_id")` returned `None` for both tables — the internal index name is `image_id_idx`, not `image_id`. Not blocking (index works), but stats call should use the correct name if we want to monitor staleness.
* Fragment count grew from 151 → 298 over ~1.5 hours (each `add()` creates a new fragment). This strongly supports Hypothesis B.

##### Hypothesis B: High fragmentation degrading all reads — CONFIRMED

**Background:** Each `add()` call in `skeet-store/src/lib.rs` (line ~151) writes a single-row `RecordBatch`, creating a new fragment each time. `compact_every_n_writes` mitigates this for `images_table`, but `scores_table` has NO compaction at all — only `images_table` is compacted in `compact()` (line ~158). Additionally, `upsert_score` (line ~334) does a `delete` then `add` on `scores_table`, each of which creates version churn. With hundreds/thousands of writes, fragment counts could be very high, causing every query to scan many small files. LanceDB recommends keeping fragment counts low (under ~100) until ~1 billion rows.

**How to prove/disprove:**
* [x] Log table statistics at startup using `table.stats()` on both `images_table` and `scores_table`
    * lancedb 0.26.2 has `Table::stats() -> Result<TableStatistics>` which returns:
        * `num_rows: usize`
        * `num_indices: usize`
        * `fragment_stats.num_fragments: usize`
        * `fragment_stats.num_small_fragments: usize`
        * `fragment_stats.lengths: FragmentSummaryStats { min, max, mean, p25, p50, p75, p99 }`
    * Also log the indices found from `list_indices()` (already called at startup for index creation check — just log the result)
    * **What to look for:** `num_small_fragments` close to `num_fragments` means severe fragmentation; `lengths.mean` of 1 means every fragment is a single row
    * If fragmented: compaction of `scores_table` is missing entirely, and `images_table` compaction threshold may be too high

**Result (2026-04-06):** Confirmed. Both tables are severely fragmented, and indices are stale.

* `images_table`: 2,617 rows across 597 fragments (all small, mean length 4, p75=1). Index: 2,022 indexed, 602 unindexed (~23% flat scan).
* `scores_table`: 2,620 rows across 2,620 fragments — every row is its own fragment (mean=max=1). `image_id_idx`: 0 indexed rows, 2,627 unindexed (100% flat scan, index completely useless). `model_version_idx`: 366 indexed, 2,261 unindexed (~86% flat scan).
* `scores_table` has no compaction at all. `images_table` compaction runs but doesn't call `OptimizeAction::All` frequently enough to keep the index current.
* Root causes: (a) every `add()` creates a single-row fragment, (b) `upsert_score` does delete+add creating churn, (c) `compact()` only optimizes `images_table`, not `scores_table`, (d) `OptimizeAction::All` rebuilds indices but the compaction threshold (100 writes) means long periods of stale indices.

**Fixes:**

1. Extend `compact` CLI + `SkeetStore::compact()` to cover all tables and optimisation actions:
    * [x] `compact()` must optimize both `images_table` and `scores_table` (currently only `images_table`)
    * [x] `compact()` must run `OptimizeAction::All` which includes both fragment compaction and index rebuild
    * [x] Log before/after stats so we can see the effect of compaction

2. Add a `compact` CronJob on k8s that runs the `compact` CLI against R2 every 30 minutes:
    * [x] Create a Dockerfile for the `compact` binary (similar to existing Dockerfiles)
    * [x] Add `build-compact` / `push-compact` just targets (similar to other container targets)
    * [x] Create `infra/k8s/compact-cronjob.yaml` as a k8s CronJob (schedule: `*/30 * * * *`, using R2 store path and secrets from pruner-deployment)

3. Reduce fragmentation at write time:
    * [x] `compact_if_needed()` should also compact `scores_table`, not just `images_table`

4. Make images_table compaction memory-safe on constrained nodes (8GB Hetzner):
    * [x] Set `target_rows_per_fragment: 500` for images_table — lance's compaction planner
      only considers fragments with `physical_rows < target_rows_per_fragment` as candidates
      ([candidacy check in plan_compaction()](https://github.com/lancedb/lance/blob/v0.20.0/rust/lance/src/dataset/optimize.rs#L693)).
      With the default 1M, all existing fragments (858 + 2022 rows) were candidates, causing
      lance to read ~5.7GB of image data into memory at once. At 500, those fragments are left
      alone while single-row fragments from `add()` are merged into ~500-row groups (~1GB).
    * [x] Set `num_threads: 1` — limits compaction to one parallel task at a time
    * [x] Set `batch_size: 64` — limits the scanner read batch size during compaction
    * [x] Run `OptimizeAction::Compact` and `OptimizeAction::Index` as separate steps (not
      `OptimizeAction::All`) to avoid index rebuild competing for memory during compaction
    * All `CompactionOptions` fields: [docs.rs/lancedb CompactionOptions](https://docs.rs/lancedb/0.26.2/lancedb/table/struct.CompactionOptions.html)
    * scores_table uses default `target_rows_per_fragment` (1M) since rows are small (~100 bytes)
    * CronJob currently suspended pending verification that the fix works; unsuspend after a
      successful manual run

##### Hypothesis C: `list_scored_summaries_by_score` does two full table scans per call — CONFIRMED

**Background:** `list_scored_summaries_by_score` (line ~451) reads ALL rows from `scores_table` (no filter), then calls `list_all_summaries` which reads ALL rows from `images_table` (selecting 7 columns but no filter), then joins in memory. This runs on every request to the feed endpoint (`skeet-feed/src/handlers.rs`, line ~117) and inspect endpoint (`skeet-inspect/src/handlers.rs`, lines ~123 and ~204). Combined with fragmentation (Hypothesis B), this could be very slow.

**How to prove/disprove:**
* [x] Add row count logging inside `list_scored_summaries_by_score` after each read
    * Log `score_map.len()` (number of score rows), `summaries.len()` (number of image summary rows), and `scored.len()` (number of matched/joined rows)
    * Use `info!` level since this method is called infrequently (on web requests, not in the hot pipeline path)
    * **What to look for:** if row counts are in the hundreds and the method takes seconds, the cost is in scanning fragmented storage, not in the in-memory join; if row counts are in the thousands, the sheer volume is also a factor
    * If confirmed: potential fixes include (a) adding a server-side join or filter, (b) caching the result with a TTL, (c) compacting both tables regularly
* [x] Add integration tests for skeet-feed (`just integ_test_feed`) to exercise the feed endpoint against a running server

**Result (2026-04-07):** Confirmed. Each feed request takes ~3.6s, dominated by two full table scans over S3:

* `scores_table`: 3,657 rows read in ~1s
* `images_table`: 3,659 summary rows (7 columns) read in ~2.6s
* In-memory join: 3,657 matched rows in ~2ms — negligible
* Total: ~3.6s per feed request

Fragmentation is already much improved from Hypothesis B fixes (22 fragments for images, 18 for scores), so the remaining cost is simply reading all ~3.7k rows per table over S3 on every request. The feed only returns 10 results, but scans everything to find them.

**Fixes:**

LanceDB does not support cross-table JOINs ([no multi-table query API](https://docs.rs/lancedb/latest/lancedb/query/struct.Query.html)). However, it does support `IN` filtering via `only_if()`. The fix is to avoid full table scans by querying in two targeted steps:

1. [ ] Query `scores_table` for top-N scores only, not all rows
    * LanceDB `Query` does not support `ORDER BY`; read scores (small rows, ~100 bytes each), sort in memory, and take top-N
    * This gives us a small set of `image_id`s (e.g. 10–50) instead of all ~3.7k
2. [ ] Query `images_table` with `only_if("image_id IN ('id1', 'id2', ...)")` using only those top-N image IDs
    * This leverages the existing `image_id_idx` scalar index for an indexed lookup instead of a full scan
    * The images_table scan (currently ~2.6s) should drop to milliseconds for 10–50 rows
    * The result is already the joined set — no separate join step needed


##### Hypothesis D: logging every 30 seconds: it's possible this is somehow blocking things (as it sits at end of pipeline) - ASSUMED DISPROVED (see below)

* [x] #1: make the 30s status logging interval configurable; check if it blocks the save stage
    * Added `--status-interval-secs` CLI arg (default: 30) passed through to save stage
    * **Does not block:** `maybe_log()` only calls `tracing::info!()` which is non-blocking

#### Benchmarking

* [x] create a minimal `bench-firehose` binary that runs the jetstream stage only for 5 mins and reports messages/sec stats
* [x] add `just bench-firehose` target and k8s deployment for running on Hetzner
* [x] extend the benchmark to have an image fetch stage i.e.
    * run for 5 minutes, collecting candidates and images (stay as-is)
      * however extend this stage so that it remembers (but doesn't fetch the images)
    * add a new stage that goes through these images one at a time, grouped by status code, measures:
      * latency of download per image
      * latency per byte

Results:
* locally (running on laptop):
```
2026-04-06T01:46:51.031976Z  INFO bench_firehose: === phase 1: firehose results ===
2026-04-06T01:46:51.032042Z  INFO bench_firehose: totals elapsed_secs=300.0 total_events=11218 total_candidates=1580 total_images=2057 candidate_pct=14.1%
2026-04-06T01:46:51.032069Z  INFO bench_firehose: throughput events_per_sec=37.4 candidates_per_sec=5.3 images_per_sec=6.9
...
2026-04-06T01:49:25.693371Z  INFO bench_firehose: === phase 2: image fetch results ===
2026-04-06T01:49:25.693424Z  INFO bench_firehose: by status status="200" count=2057 avg_latency_ms=75.1 min_latency_ms=27.2 max_latency_ms=1121.5 avg_bytes=89007 total_bytes=183087438
```
* hetzner cluster (running shared with everything else):
```
2026-04-06T01:49:03.285757Z  INFO bench_firehose: === phase 1: firehose results ===
2026-04-06T01:49:03.285790Z  INFO bench_firehose: totals elapsed_secs=300.0 total_events=11128 total_candidates=1574 total_images=2071 candidate_pct=14.1%
2026-04-06T01:49:03.285804Z  INFO bench_firehose: throughput events_per_sec=37.1 candidates_per_sec=5.2 images_per_sec=6.9
...
2026-04-06T01:53:11.084885Z  INFO bench_firehose: === phase 2: image fetch results ===
2026-04-06T01:53:11.084916Z  INFO bench_firehose: by status status="200" count=2071 avg_latency_ms=119.6 min_latency_ms=6.4 max_latency_ms=5800.7 avg_bytes=88752 total_bytes=183806003
```

Conclusions:
* jetstream delivers ~37 posts/sec (filtered to `app.bsky.feed.post` at connection level)
* ~15% of posts have images, giving ~5–6 candidates/sec (~7 images/sec, ~1.3 images per candidate)
* results are nearly identical across laptop and Hetzner, confirming the rate is set by Bluesky's post volume, not our compute or network
* this sets the input ceiling for the pruner pipeline: at ~6 candidates/sec, each candidate must be processed in under 1/6s ≈ 170ms on average to keep up
* image fetches (full download, not just TTFB): all 200s (no errors), ~89KB average per image
    * laptop: avg 75ms, min 27ms, max 1.1s — fetched ~2k images in ~2.5 mins
    * hetzner: avg 120ms, min 6ms, max 5.8s — fetched ~2k images in ~4 mins
    * laptop has lower avg latency than Hetzner (possibly CDN edge proximity)
    * at ~75–120ms per image sequentially, fetching 7 images/sec would require parallelism (sequential can only do 8–13/sec)

Optimisations to consider:
* pruner image stage currently downloads + classifies images sequentially per candidate — at 120ms/image on Hetzner, a 2-image candidate takes ~240ms, already over the 170ms budget before classification even runs
* could overlap image downloads within a candidate (fetch all images concurrently, classify as they arrive)
* could pipeline across candidates: start fetching the next candidate's images while classifying the current one
* image classification (face/skin detection) is CPU-bound and probably fast compared to the network fetch, so the fetch is the bottleneck to target first

#### Visibility

* [x] extend `skeet-feed` on fly.io to send opentelemetry data to honeycomb, so that we can examine runtimes there
* [x] add any missing instrumentation to the main flows of `skeet-feed`
* [x] #1: add channel depth and per-stage throughput logging to the pruner pipeline
    * Added `PipelineCounters` (atomic counters per stage) and `ChannelMonitors` (sender clones for depth via `max_capacity - capacity`) in `pipeline.rs`
    * Each stage increments its counter; save stage logs throughput rates and channel depths alongside existing status every 30s

#### Optimisations (act on information from above first)

* [ ] live-refine: dispatch OpenAI calls in parallel (currently sequential)
* [ ] live-refine: batch-save scores to lancedb to reduce fragmentation
