# Architecture

## Background

This project recreates [the original Twitter-based selfie finder](https://github.com/mikemoraned/selfies) ([blog post](https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/)) using Bluesky instead of Twitter and modern technologies.

The original project scanned Twitter's firehose, applied face detection (via OpenIMAJ), and looked for selfie-like compositions (face in foreground border, landmark in background). It had a ~0.1% hit rate and encountered challenges like porn, screenshots, and false positives from inanimate objects resembling faces.

## Target Architecture

- **skeet-prune** — continuously listens to the Bluesky firehose and applies fast, approximate checks to discard candidates that can't possibly match, then stores surviving images in the skeet-store
- **skeet-refine** — applies more expensive LLM-based scoring to pruned candidates, assigning each a quality score
- **skeet-store** — stores found skeets in an S3-compatible store, in tables, managed as [LanceDB](https://lancedb.com) tables.
- **skeet-feed** — Bluesky feed generator (AT Protocol feed skeleton) that serves the top-scored images as a custom feed. Deployed on Fly.io with OpenTelemetry tracing to Honeycomb.
- **skeet-inspect** — HTTP service that reads from the store and surfaces all found skeets for inspection

## Constraints, Trade-offs and Technology Choices

- **Rust first:** all code should be in Rust where possible
- **ML:** use existing models or Rust libraries for face-detection and landmark identification
  - Non-Rust ML models are OK if really required
- **Burn:** use [Burn](https://burn.dev) ([GitHub](https://github.com/tracel-ai/burn)) for running ML models
- **Sampling:** processing at line-speed may not be possible — sampling is fine. Simple parts (e.g. checking if a message contains an image) should be inline with receiving a message.

## Prune and Refine Pattern

The pipeline follows a **prune-and-refine** pattern:

1. **Prune** (`skeet-prune`): fast, approximate checks that discard the vast majority of candidates. This stage runs inline with the firehose and uses cheap operations — face detection, skin detection, and metadata filtering — to reduce the stream to a sub-1% hit rate. Biased towards recall: a small percentage of false positives are acceptable because they will be caught in the refine stage.

2. **Refine** (`skeet-refine`): expensive, precise scoring applied only to the candidates that survive pruning. Uses an LLM to evaluate how well each image matches the target intent (selfie with a recognizable landmark). Produces a score between 0.0 and 1.0.

Configuration for each stage lives in `config/`:
- `config/prune.toml` — thresholds for face area, skin percentages
- `config/refine.toml` — LLM provider, model name, and scoring prompt

Both configs produce a `ModelVersion` (a short hash of their contents) used to track which version of the config was active when an image was processed or scored.

## Testing

- **Blocklist as test fixture:** the `blocklist/` directory captures real examples of skeets that should have been blocked (adult content, `!no-unauthenticated` authors, etc.). Each entry has a `getPostThread` JSON snapshot. The blocklist is *not* used in the live pipeline — it exists purely to drive integration tests that verify the metadata filtering logic works correctly.
