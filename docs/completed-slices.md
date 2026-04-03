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
