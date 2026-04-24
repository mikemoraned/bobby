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

##### Observations

###### 24th Apr

The `skeet-feed` sends about 2.5K Class B operations. This kinda makes sense now in that there is a background job that refreshes once a minute. 

`skeet-prune` and `skeet-live-refine` seems to both do a *lot* of `get` and `get_range` requests (both send up to 30K per minute of each, for a period of about 4 minutes each). During this time other operations like `head`,`list` and `put` are tiny (10's per minute) I can sort-of understand why live-refine might need to do a lot of gets to get an image (though would be good if it's not lots of requests), however I don't see why pruner would have to.

#### Idea: Remove inline compaction in favour of the cron job

The `compact` cron job already runs every 10 minutes against all tables. The `compact_every_n_writes` mechanism duplicates this inline, blocking the save path and generating large GET/GET_RANGE bursts against R2 during each run.

* [x] remove `compact_every_n_writes` from `StoreArgs` and `SkeetStore` entirely
* [x] remove the `compact_if_needed` call sites in `lib.rs` and `scores.rs`
* [x] remove the `writes_since_compact` counter from `SkeetStore`

#### Idea: Batch image fetches in live-refine

`live_refine.rs` fetches images one at a time via `get_by_id` inside a loop, generating O(N) separate R2 queries each returning a full `StoredImage` (~4MB: original + annotated PNG blobs). Live-refine only needs the original image for scoring.

* [ ] replace the per-image `get_by_id` loop (`live_refine.rs:78-97`) with a single `store.get_by_ids(&batch_ids)` call before dispatching the scoring batch
* [ ] make `annotated_image` optional in `StoredImage` (e.g. `Option<DynamicImage>`), and add a fetch mode or separate query path that skips the `annotated_image` column — so callers like live-refine that don't need it don't pay the R2 cost

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

#### Idea: Switch to notification-listening queue for live-refine

* [ ] rather than polling the remote store for recently-updated images that have been pruned, the `pruner` and `live-refine` clis can communicate via a notification queue that says when an image candidate has been found.

#### Idea: put in place some sort of caching of Lancedb R2 lookups

* [ ] ...

#### Idea: run LLM models in batch mode

* [ ] ...

#### Idea: run a local model inside k8s cluster (via ollama)

* [ ] ...
