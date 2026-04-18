# Completed Slices

## Slice 1: A random local feed

Built the end-to-end pipeline:

- **skeet-store** crate with LanceDB `images_v1` table (image_id, skeet_id, image_data as PNG, discovered_at, original_at)
- **skeet-finder** listens to live Bluesky firehose via `jetstream-oxide`, finds posts with images (`app.bsky.embed.images` and `recordWithMedia`), randomly selects 1% of images, downloads from CDN, saves to store. Run via `just find`.
- **skeet-feed** web UI showing embedded skeets from the store using cot.rs and Bluesky's embed.js. Run via `just feed` (http://127.0.0.1:8000/).

## Slice 2: Finding faces

Added face detection and archetype matching:

- Integrated YuNet ONNX face detection model for frontal face detection
- Defined Archetype enum (TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT, BOTTOM_RIGHT) based on face position in image quadrants
- Schema evolved to `images_v2` then `images_v3` (added annotated images column)
- Added annotated image generation (bounding boxes, crosshairs) with `/skeet/{image_id}/annotated.png` endpoint
- Introduced `StoredImageSummary` for memory-efficient listing; feed shows 50 most recent
- Added zone-based classification with 5 zones (4 quarters + central); faces in central zone rejected
- Config-driven examples via `expected.toml` with `libtest-mimic`; tunable face size thresholds (10%–60%) in `archetype.toml`
- Rejection reasons: FaceTooSmall, FaceTooLarge, TooManyFaces, face in central zone
- `classify_examples` diagnostic CLI for tuning

## Slice 3: Removing porn (false positives)

- Refactored `skeet-finder` main.rs into sub-modules (firehose handling vs classification)
- Added indicatif progress bar (spinner, runtime, skeets/images seen, images saved, hit-rate)
- Filtered adult content: skeets with `Adult Content` flag or `!no-unauthenticated` author labels
- **skin-detection** crate: ML-based pixel-level skin classification accounting for different ethnicities
  - Inclusion filter: face bounding boxes must contain sufficient skin (`min_face_skin_pct` in archetype.toml)
  - Exclusion filter: skin outside face limited (`max_outside_face_skin_pct` in archetype.toml)
- Skin detection mask used in annotated images
- Integration tests driven by blocklist of AT URLs for adult content filtering

## Slice 4: Removing text (false positives)

- Added per-rejection-reason counters with percentages in skeet-finder output
- **text-detection** crate: OCR-based glyph counting (multi-language support)
- Rejection::TooMuchText for images exceeding `max_glyphs_allowed` (default: 5) in archetype.toml
- Refactored crate responsibilities: leaf crates (text-detection, skin-detection, face-detection) only depend on `shared`; classification logic lives in skeet-finder
- Generic types (Rect, translate) moved to external crates
- Annotated images show text bounding boxes; `detected_text` column added to images table
- `classify-examples` updated to show detected text
- Feed UI shows config version and detected text columns
- Added exemplar metadata to expected.toml for key example images
- Verified adult-based filtering with `metadata-dump` CLI and `add_to_blocklist` CLI
- Extended `blocked_labels()` to check both post labels (porn) and author labels (!no-unauthenticated)

## Slice 5: Meta: Split TODO.md into Claude Code memory hierarchy

Restructured project documentation for Claude Code workflows. Created `CLAUDE.md` at project root, `.claude/rules/` with Rust and Python rule files (with `paths:` frontmatter), and split the monolithic `TODO.md` into `docs/architecture.md`, `docs/current-slice.md`, `docs/next-slices.md`, and `docs/completed-slices.md`. Deleted the original `TODO.md` with no information loss.

## Slice 6: Tweak recognition parameters and filtering

Refined face position classification and text/pre-filtering:

- Replaced the old Archetype enum with a rigorous 3×3 Zone grid (9 zones: TOP_LEFT, TOP_CENTER, TOP_RIGHT, CENTER_LEFT, CENTER_CENTER, CENTER_RIGHT, BOTTOM_LEFT, BOTTOM_CENTER, BOTTOM_RIGHT). Zones are 2×2 units on a 4×4 grid overlay. Successful matches limited to corner and side zones; TOP_CENTER and CENTER_CENTER rejected — catches faces previously slipping through as false positives.
- Converted `Archetype` usages to `Option<Zone>`, requiring an images table schema migration.
- Improved pre-filtering: detect and block re-skeets/quoted posts with author opt-out labels.
- Split `metadata_dump` CLI into `image_metadata_dump` and `at_metadata_dump` (shared `metadata` module) for better debugging.
- Switched text filtering from glyph count to text area percentage of the image, with new parameters in `archetype.toml`, reducing false positives from overlaid text.

## Slice 7: Make version available that can run on different machines

Moved storage to the cloud and added observability:

- **Cloudflare R2 storage**: `SkeetStore::open` now accepts S3 URIs with storage options via `StoreArgs` (clap-derived). All binaries (`finder`, `feed`, `validate-storage`, `image-metadata-dump`) migrated to `StoreArgs`. R2 credentials stored in 1Password, accessed via Justfile helpers.
- **SSE-C encryption**: data at rest encrypted with customer-provided 256-bit AES key via S3-compatible SSE-C headers, transparent to LanceDB operations. Key stored in 1Password.
- **Tracing and observability**: switched to `tokio-tracing` with daily rolling file appender (ANSI disabled for file output) and optional stderr output. Added `#[instrument]` annotations across `SkeetStore` methods, `persistence::save`, and feed handlers for performance visibility.
- **OpenTelemetry**: optional OTLP exporter layer activated by `OTEL_EXPORTER_OTLP_ENDPOINT` env var; when absent, a warning is logged and OTEL is disabled. Configured for Honeycomb via Justfile `*-r2` rules with ingest key from 1Password.
- **tokio-console**: opt-in via `--tokio-console-port` CLI arg on `finder` and `feed`. Uses `console_subscriber::ConsoleLayer::builder().init()` as a standalone subscriber — file and OTEL layers are disabled in this mode due to a known incompatibility between `ConsoleLayer` and `fmt::Layer` span tracking.
- **Refactoring**: eliminated redundant face detection in `classify_image`, deduplicated excluded-labels constants, fixed `ImageId::as_str()` conventions, extracted shared tracing setup to `shared::tracing`, embedded `StoredImageSummary` inside `StoredImage`.

## Slice 8: Minimal qualitative scoring on top of Envelope filtering

Added scoring, robustness, and terminology refactoring across the pipeline:

- **Store improvements**: content-addressable `ImageId` (MD5 hash), BTree scalar index on `image_id`, deduplication on save, and `read_consistency_interval(Duration::ZERO)` for strong cross-process consistency.
- **Pipeline robustness**: split firehose into `filter` and `save` stages connected by a channel; added local fallback store (dead-letter queue) for failed remote saves with a `redrive-r2` CLI for reconciliation. Improved firehose connection reliability (random endpoint selection, connect/receive timeouts, thumbnail downloads). Tuned LanceDB with generous HTTP timeouts, auto-compaction every N writes, a `compact` CLI, and raised `client_max_retries` to 3.
- **Secrets management**: moved secrets from CLI args to env vars via `op run --env-file bobby.env`.
- **Content filtering fixes**: fixed status counter to track rejected images (not reasons); fixed adult content and author opt-out filtering by splitting `filter_stage` into `filter_meta_stage` and `filter_image_stage` with integration tests proving correctness on real firehose code paths. Added `/add-to-blocklist` skill.
- **skeet-scorer → skeet-refine**: introduced LLM-based image scoring (via OpenAI, using generic Rust crates) with `train`, `rescore`, and `live-score` CLIs. Config-versioned `refine.toml` with `RefineModelConfig` ensures scores track which model version produced them.
- **Terminology refactor**: renamed `skeet-finder` → `skeet-prune` and `skeet-scorer` → `skeet-refine` to follow prune-and-refine pattern; `archetype.toml` → `config/prune.toml`, `model.toml` → `config/refine.toml`. Documented pattern in `architecture.md`.
- **Debugging & UX**: `summarise` CLI and `SkeetStoreSummary` on feed homepage; feed split into `latest` (all skeets) and `best` (scored, ordered by score) pages with homepage links.

## Slice 9: "Bobby Dev" Custom Feed in Bluesky

Built a live Bluesky Custom Feed for dev testing, with supporting refactors:

- **Refactors**: renamed `skeet-feed` → `skeet-inspect` (inspection UI) and renamed pages (`latest` → `pruned`, `best` → `refined`) with unified page format. Freed up the `skeet-feed` name for the actual feed.
- **Text detection removal**: added `RejectionCategory` analysis showing text-based rejection was sole cause only 1% of the time. Removed the `text-detection` crate, associated models, and all references entirely.
- **New `skeet-feed`**: a cot.rs web app deployed to Fly.io (`bobby-staging.fly.dev` / `bobby-staging.houseofmoran.io`) serving the Bluesky feed skeleton API. Connects to the remote R2 store and surfaces the top 10 skeets scored above 0.5 from the last 48 hours. Includes `deploy_staging`, `test_webapp`, and `test_staging` Justfile recipes, plus a helper to sync `bobby.env` secrets with Fly.io.
- **Feed registration**: wrote a Rust CLI to register the Custom Feed with Bluesky (inspired by `skyfeed` crate and official docs).
- **Refine improvements**: live-refine now prioritises most recently discovered images, scores within a time budget (matching the polling interval) before re-checking for newer arrivals, and uses a `model_version` scalar index on the scores table for efficient unscored-image queries.

## Slice 10: Running pruning/refining remotely on Hetzner

Moved the pruner and live-refine workloads from local machines to a single-node k3s cluster on Hetzner Cloud ARM (CAX21 in fsn1), provisioned via `hetzner-k3s`:

- **Cluster provisioning**: cluster config at `infra/bobby-cluster.yaml` with SSH keys and API token stored in 1Password. Just recipes handle key export/cleanup automatically. Added `just cluster-create`, `just cluster-delete`, and `just cluster-prerequisites`.
- **Container images**: multi-stage Dockerfiles for both `pruner` (includes ONNX models and BPK weights with path baked into the binary) and `live-refine`. Built for `linux/arm64` with `RUSTFLAGS="-C target-cpu=neoverse-n1"` to avoid fp16 assembly errors. Published to GitHub Container Registry via classic PAT.
- **Secret injection**: replaced local `op run --env-file` with the 1Password Kubernetes Operator. Six `OnePasswordItem` CRDs sync R2 credentials, SSE-C key, OpenAI API key, and Honeycomb API key to k8s Secrets. Honeycomb headers constructed via k8s env var interpolation.
- **Deployments**: k8s deployment manifests for both services with `imagePullSecrets` for GHCR, OTEL telemetry to Honeycomb with `deployment.environment=hetzner` resource attribute, and `OTEL_SERVICE_NAME` per service.
- **Operations**: umbrella recipes (`cluster-deploy`, `cluster-restart-*`, `cluster-logs-*`, `cluster-status`) for common remote operations. Full setup/teardown documented in `docs/remote-setup.md`.
- **Justfile decomposition**: split the 244-line monolithic Justfile into `just/store.just`, `just/feed.just`, `just/container.just`, and `just/cluster.just` using just's `import` feature. Exported `KUBECONFIG` as an environment variable to eliminate 11 manual prefixes.

## Slice 11: Improve Rust compile times, both locally and remotely

Reduced compile times and streamlined the Docker build pipeline:

- **Dependency audit**: used `cargo-machete` to remove unused deps (`tracing-subscriber` from skeet-store, `tokio` from shared, `reqwest` from skeet-refine). Added false-positive ignores for `face-detection` build-time deps.
- **Feature pruning**: ran `cargo-features-manager` across the workspace; switched high-impact deps (`reqwest`, `tokio`, `serde`) to `default-features = false` with explicit feature selection. Centralised shared dependency versions in `[workspace.dependencies]`.
- **cargo-chef caching**: restructured all service Dockerfiles into three-stage builds (planner/builder/runner) using `lukemathwalker/cargo-chef:latest-rust-1-bookworm`. Source-only changes now get a cache hit on the dependency compilation layer.
- **BuildKit cache mounts**: added `--mount=type=cache` for cargo registry and git dirs on both `cargo chef cook` and `cargo build` steps.
- **Shared base image (attempted and reverted)**: tried extracting common Docker stages into `bobby-chef` and `bobby-runner` base images. Reverted due to multiarch builder complexity, 5GB chef image too large for GHCR, and builder driver incompatibilities. Kept self-contained Dockerfiles with the good parts inline.
- **fly.io pre-built images**: switched `fly.staging.toml` from building on fly.io to pulling pre-built amd64 images from GHCR. GHCR packages made public for unauthenticated pulls.
- **Build config**: moved architecture-specific RUSTFLAGS (`-C target-cpu=neoverse-n1` for ARM) into `.cargo/config.toml` per-target sections. Added `.dockerignore` excluding `target/`, `store/`, `logs/`, and other large dirs.

## Slice 12: Optimisations of pruning, refining, and feeding

Systematic investigation and resolution of performance bottlenecks across the pipeline:

- **LanceDB fragmentation (Hypothesis confirmed)**: every `add()` created a single-row fragment; `scores_table` had zero compaction. Fixed by extending `compact()` to cover all tables, adding a k8s CronJob for periodic compaction, and tuning `CompactionOptions` (target 500 rows/fragment, single-threaded, batch size 64) to stay within 8GB Hetzner memory.
- **Full table scans on feed requests (Hypothesis confirmed)**: `list_scored_summaries_by_score` scanned all rows from both tables (~3.6s per request). Replaced with a two-step query: fetch top-N scores, then indexed lookup of only those image IDs. Also added a version-gated scores cache in `SkeetStore`.
- **Scalar index usage (Hypothesis disproved)**: `get_by_id` was already using the scalar index correctly; the slow queries were caused by fragmentation.
- **Benchmarking**: built `bench-firehose` binary measuring firehose throughput (~37 posts/sec, ~7 images/sec) and image download latency (75–120ms avg). Established the 170ms/candidate processing budget.
- **Pipeline throughput**: parallelised image downloads within candidates (+15–20%), added multi-worker image stage (`--image-workers`, default 2) eliminating the image classification bottleneck. Pipeline now keeps up with the firehose.
- **Live-refine**: parallelised OpenAI calls (`--concurrency`, default 4) and batch-upserted scores to reduce fragmentation.
- **Visibility**: added OpenTelemetry to skeet-feed on fly.io, per-stage pipeline counters with channel depth monitoring, slow-query plan logging (>100ms threshold).
- **Feed caching**: added a read-through `FeedCache` in skeet-feed with 5-minute staleness window and 1-minute background refresh, reducing feed response times from ~8s to near-instant for cached results. Used tokio's `start_paused`/`advance` for deterministic time-based tests.
- **Test infrastructure**: extracted shared test helpers (`make_record`, `open_temp_store`, etc.) into `skeet-store::test_utils` behind a `test-helpers` feature flag, deduplicating across four test files.

## Slice 13: Add /admin area in skeet-feed for manual quality appraisal of skeets and images

Built a full admin area with manual appraisal capabilities and GitHub OAuth authentication:

- **Domain types**: added `Band` enum (Low/MediumLow/MediumHigh/HighQuality) with score-based classification, visibility rules, and an `Appraiser` enum (GitHub username or LocalAdmin) with wire-format serialisation.
- **Shared web crate**: extracted `skeet-web-shared` for store middleware, shared view types, vendored htmx, and base layout template shared between skeet-inspect and skeet-feed.
- **Storage**: cursor-paged listing (`list_summaries_page`), manual appraisal tables for both skeets and images (`manual_skeet_appraisal_v1`, `manual_image_appraisal_v1`) with set/get/clear/list operations preserving appraiser identity.
- **Effective band logic**: computes per-image and per-skeet effective bands combining automatic scores with manual overrides; one bad image taints the whole skeet; manual skeet override wins over automatic. Replaced `min_score` config with band-based visibility.
- **Feed integration**: `FeedCache::refresh()` loads manual appraisals; `get_feed_skeleton` uses effective-band visibility. Added `Cache-Control: no-cache` support to force refresh, with `Last-modified` header showing cache freshness.
- **Home view** (`/`): displays currently-visible feed items sorted best-to-worst, with appraisal controls shown when user is logged in as admin.
- **Admin view** (`/admin`): cursor-paged infinite scroll via htmx, showing all stored items with thumbnails, scores, bands, and per-item band selector buttons (4 bands + clear). Supports both skeet and image appraisal sub-views.
- **GitHub OAuth**: full OAuth flow (`/auth/login`, `/auth/callback`, `/auth/logout`) with CSRF protection, username allowlist from `BOBBY_ADMIN_USERS` env var, and session-based role management. `--local-admin` flag for local development. Split env files into `bobby-local.env` and `bobby-staging.env`.
- **Auth guard**: handler-level `AppraiserExtractor` checks both static extensions (local-admin) and session (OAuth); unauthenticated requests redirect to login, non-allowlisted users get 403.
- **Testing**: comprehensive integ tests for OAuth flow (mocked GitHub), admin access control, CSRF rejection, feed visibility after appraisal mutations, and paging.

## Slice 14: Property-based tests for value types + Mutation-testing for coverage check

Adopted `proptest` for property-based testing across value types and introduced `cargo-mutants` for mutation testing to verify test coverage:

- **Property-based tests**: converted example-based tests to property-based for `Score` (validity, parse/display roundtrip, ordering), `Percentage` (refactored from panic to `Result`, validity, equality), `ImageId` V1/V2 (roundtrip, content-addressability), `SkeetId` (roundtrip, hash consistency), `Band` (totality, monotonicity, parse/display), `Rejection` (exhaustive roundtrip), `Zone` (roundtrip), `Appraiser` (roundtrip, rejection), `PruneConfig` (hash consistency), `DiscoveredAt` (time arithmetic), and effective band logic (manual override semantics).
- **Mutation testing infrastructure**: migrated test runner to `nextest` for faster execution, configured via `.cargo/mutants.toml`. Added `just mutants-on-diff` recipe to run `cargo mutants --in-diff` against the main branch diff.
- **Mutation testing results**: systematically ran `cargo mutants` on shared (36→6 missed), skeet-store (134→92), and skeet-feed (36→24). Added targeted unit and integration tests for health.rs (LanceDB stats), feed_cache (staleness boundary, refreshed_at), handlers (wants_no_cache, set_last_modified_header), store operations (get/delete/compact/summarise), admin views (image view, appraise response body), and home page.
- **Detection crate tests**: added high-level public API tests for skin-detection (`detect_skin` dimensions/marking, `skin_pct_in_rect`/`skin_pct_outside_rect`) and face-detection (`area_pct_known_value`, `annotate_image` rendering). Deferred deeper algorithm-level mutation coverage to a future slice using test dataset comparisons.
- **Cleanup**: removed dead code (`PartialEq<&str> for Nsid`), added distinctness tests for Band labels/descriptions, and ensured all 230 tests pass with clippy clean.
