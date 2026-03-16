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
