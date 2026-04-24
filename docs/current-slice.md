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

#### Idea: Only update feed cache on version change

As of 24th Apr, from looking at the metrics graphs, the `skeet-feed` sends about 2.5K Class B operations. This kinda makes sense now in that there is a background job that refreshes once a minute. Ultimately it'd be good for this to be more of a push-on-change approach, where a central cache is updated when something has changed about scoring or similar. However, for now, I think we can have a different approach i.e.

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
