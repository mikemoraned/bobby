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
        * [ ] replace bespoke plan string parsing in `trace_analysis.rs` with a typed `QueryPlan: Serialize + Deserialize` struct defined in `lancedb_utils.rs`; parse the raw `explain_plan` output once at log time, emit as JSON in the `plan` event attribute, deserialize in `trace_analysis.rs` — removes `extract_field` / `plan_summary` string hacks

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


##### Observations

###### 24th Apr

The `skeet-feed` sends about 2.5K Class B operations. This kinda makes sense now in that there is a background job that refreshes once a minute. 

`skeet-prune` and `skeet-live-refine` seems to both do a *lot* of `get` and `get_range` requests (both send up to 30K per minute of each, for a period of about 4 minutes each). During this time other operations like `head`,`list` and `put` are tiny (10's per minute) I can sort-of understand why live-refine might need to do a lot of gets to get an image (though would be good if it's not lots of requests), however I don't see why pruner would have to.

###### 25th Apr

Tempo spike confirmed query plan data is available in traces. Two slow queries observed on every `list_unscored_image_ids_for_version` tick:

* `list_unscored:scored_ids` — 1.51s. Plan: `LanceRead` on `images_score_v2.lance`, projection `[image_id]`, 4 fragments, uses `ScalarIndexQuery` on `model_version_idx`. Slow despite the index.
* `list_all_image_ids_by_most_recent` — 2.04s. Plan: `LanceRead` on `images_v6.lance`, projection `[image_id, discovered_at]`, **66 fragments, no filter, no index** — full table scan every tick. This is almost certainly the dominant source of `get`/`get_range` R2 traffic from `skeet-live-refine`.

Checked the `compact` cron job to test the hypothesis that fragments were piling up. They are not — the cron is healthy (running every 10 min, completing in ~60–140s). The fragment count is **stuck at ~64 by design**: `skeet-store/src/compact.rs:50` sets `target_rows_per_fragment: 500` (low on purpose, because each row carries a ~2MB PNG blob and the lance default of 1M would OOM the compactor). Lance's planner only flags fragments with `physical_rows < 500` as candidates, so anything ≥500 rows is left alone forever. Each cron run merges only the small stragglers (e.g. "365 rows across 2 fragments" + "4 rows across 4 fragments") and lands back at ~64 fragments (mean=433, p50=500, p99=2022). Implications:

* Compaction is **not the lever** — pushing `target_rows_per_fragment` up would re-introduce OOM risk.
* The full scan in `list_all_image_ids_by_most_recent` will remain expensive as long as it scans every fragment.
* Our `RECOMMEND: compact: 64 small fragments (>10 threshold)` health line is misleading — it uses lance's default smallness threshold, not our chosen 500. (Worth fixing separately.)
* This makes the case stronger for both *Idea: reduce cost of polling in live-refine* (version-snapshot early-abort skips the scan when nothing changed) and *Idea: make `list_all_image_ids_by_most_recent` use `discovered_at_idx`* (let lance scan the index instead of every fragment).

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

* [ ] update `SkeetStore` to have a `version_snapshot` method which returns a `HashSet<Version>` where
    * `Version` is a struct with a `name` and `tag`
        * `name` is the name of the underlying table
        * `value` is an opaque identifier capturing the version of the table
    * this `value` should be a `String` to keep non-coupled to the underlying implementation, but which should be derived from the `version` of each underlying lancedb table
* [ ] update the `skeet-feed` cache so that it still runs once a minute but functions as follows when it wants to test if cache needs updated:
    1. fetch `version_snapshot`
    2. filter `HashSet<Version>` down to only the `name`'s it depends to invalidate the cache:
        * so, for example, it is only a change in appraisals or image scores that should effect the cache; changes to images or skeets does not affect it
    3. (assuming this `HashSet<Version>` has been previously saved on the cache) compare those against what has just been found
    4. if they are different then proceed as now in invalidating and updating the cache
* [ ] we can also remove the staleness check as this method should mean we don't need it anymore
* [ ] all of the above should be down in a failing-test-first way as we are introducing more complexity here

The outcome of this should be that we only incur the cost of updating the in-memory cache when something has changed.

#### Idea: reduce cost of polling in live-refine

Every poll tick, `live-refine` runs `list_unscored_image_ids_for_version`, which scans both `images` and `scores` tables — reading LanceDB's arrow fragment files from R2 to filter IDs. This generates `get`/`get_range` calls even though no image blobs are fetched. If unscored IDs are found, `get_originals_by_ids` then fetches the actual image data (~4MB per image). The ID scans are paid every tick regardless of whether anything has changed.

`SkeetStore` will expose a `version_snapshot` as part of "Idea: Only update feed cache on version change". We can use the `images` table version from that snapshot as a cheap early-abort: if the table version hasn't changed since the last tick, no new images were committed and the expensive scan can be skipped entirely. `table.version()` is already used in `cached_scores()` and is a lightweight manifest read — not a scan.

We'll do this in stages:
* [ ] (observation) emit an OTel gauge from `SkeetStore` reporting the observed `version` for each table (label `table` ∈ {`images`, `scores`, ...}), updated on each access. This lets us see in Grafana how often the `images` table version actually changes per minute — if it changes every tick, the early-abort optimization gives no benefit and we should reconsider before building it.
* [ ] (prerequisite) "Idea: Only update feed cache on version change" is implemented, giving us `version_snapshot` on `SkeetStore`
* within `skeet-refine`, separate polling from dispatch:
    * [ ] extract the poll-and-fetch step from `live_refine.rs` into a `PollingImageSource` struct in `skeet-refine/src/`:
        * holds `store: Arc<SkeetStore>`, `model_version: ModelVersion`, and `last_images_version: Option<String>` as state between ticks
        * exposes an async `fetch(&mut self) -> Result<Vec<(ImageId, DynamicImage)>, ...>` method
        * on each call: fetch `version_snapshot()`, extract the `images` table version, and return `Ok(vec![])` immediately if unchanged since last call
        * if changed: run `list_unscored_image_ids_for_version` + `get_originals_by_ids`, update `last_images_version`, and return the candidates
    * [ ] update `live_refine.rs` main loop to call `source.fetch()` instead of doing the query inline; the dispatch half (batching, `buffer_unordered` scoring, `batch_upsert_scores`) does not change

##### Variation: hold `last_discovered_at` and push it down as a filter

The version-snapshot above is binary (changed / not changed). When the table *has* changed, we still scan every fragment looking for unscored ids. A finer variation: also remember the maximum `discovered_at` that `PollingImageSource` has seen scored, and pass it back into the store as a `WHERE discovered_at > last_discovered_at` filter on the next tick. This composes with *Idea: make `list_all_image_ids_by_most_recent` use `discovered_at_idx`* — that filter is exactly the predicate the BTree on `discovered_at` can satisfy. Result: when a tick does run, it pays a BTree range read instead of a 64-fragment scan.

* [ ] extend `PollingImageSource` with `last_discovered_at: Option<DateTime<Utc>>` state, updated to the max `discovered_at` of the candidates returned each tick
* [ ] add a `since: Option<DateTime<Utc>>` parameter to `list_unscored_image_ids_for_version` (and through to `list_all_image_ids_by_most_recent`) — when `Some`, push down a `discovered_at > <ts>` filter; when `None`, behave as today
* [ ] handle the cold-start / restart case: in-memory state means a fresh pod takes one full scan to bootstrap `last_discovered_at`; that's acceptable, no persistence needed

**Edge case to think through before building:** if scoring an image fails (e.g. `Completion`/`ParseScore` errors in `LiveRefineMetrics`), the image stays unscored but its `discovered_at` is in the past — a strictly-monotonic `last_discovered_at` cutoff would never retry it. Mitigations: (a) only advance `last_discovered_at` to the max `discovered_at` of *successfully scored* images, leaving stragglers in-window; or (b) keep the cutoff but run a periodic full reconciliation pass (e.g. once an hour) to catch dropped images. (a) is simpler and probably sufficient.

#### Idea: make `list_all_image_ids_by_most_recent` use `discovered_at_idx`

The query in `lib.rs:286-307` selects `[image_id, discovered_at]` with no `WHERE`, no `ORDER BY`, and no `LIMIT`. The trace plan confirms lance falls back to a full `LanceRead` over all 64 fragments — even though a `discovered_at_idx` BTree exists (created in `open.rs:87-95`). That index already contains both columns the projection needs, so a covering index scan is possible in principle.

The opportunity: with the fragment count pinned at ~64 by the deliberate `target_rows_per_fragment=500` setting (see 25 Apr observation), per-tick full scans will not get cheaper through compaction. Routing this query through the index instead of the data files is the only way to reduce the per-fragment R2 ops *without* changing the polling cadence. Combined with *Idea: reduce cost of polling in live-refine*, this would mean: when a tick *does* run, it pays index cost (one or two get_range per BTree page) instead of fragment cost (per-fragment overhead × 64).

* [ ] confirm via a local repro or trace that adding `ORDER BY discovered_at DESC` (and ideally a `LIMIT` matching the realistic per-tick batch) lets lance plan a `ScalarIndexQuery` on `discovered_at_idx` instead of `LanceRead` on the data files
    * lancedb query API: `query.order_by(...)` / `query.limit(...)` — needs verifying; the rust API surface for `order_by` may not be exposed directly and might require `nearest_to`-style helpers or a SQL filter
    * if lancedb's rust API does not expose `ORDER BY`, fall back to relying on the `WHERE discovered_at > <cutoff>` form (covered by the BTree) and then sorting in-process
* [ ] adjust the call site in `scores.rs:127` so callers pass through the desired ordering/limit without `list_all_image_ids_by_most_recent` having to load every row
    * `list_unscored_image_ids_for_version` is the only caller; it currently sorts the entire table by `discovered_at` so it can prefer recent unscored images — but it doesn't actually need every id, only the most-recent-N unscored
* [ ] verify in the trace plan that the new shape uses the index (look for `ScalarIndexQuery(discovered_at_idx)` or similar in `explain_plan`) and that R2 op counts during the tick drop accordingly

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

#### Idea: put in place some sort of caching of Lancedb R2 lookups

* [ ] ...

#### Idea: run LLM models in batch mode

* [ ] ...

#### Idea: run a local model inside k8s cluster (via ollama)

* [ ] ...
