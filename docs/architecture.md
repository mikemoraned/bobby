# Architecture

## Background

This project recreates [the original Twitter-based selfie finder](https://github.com/mikemoraned/selfies) ([blog post](https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/)) using Bluesky instead of Twitter and modern technologies.

The original project scanned Twitter's firehose, applied face detection (via OpenIMAJ), and looked for selfie-like compositions (face in foreground border, landmark in background). It had a ~0.1% hit rate and encountered challenges like porn, screenshots, and false positives from inanimate objects resembling faces.

## Target Architecture

The system is a set of small Rust services split by concern: a pruner and refiner produce the data, a publisher decides what's worth showing, and two web apps serve it. They coordinate through two shared stores — the LanceDB `skeet-store` (durable data) and a Redis instance (the published lists) — rather than talking to each other directly.

- **skeet-prune** — continuously listens to the Bluesky firehose (via Jetstream) and applies fast, approximate checks to discard candidates that can't possibly match, then stores surviving images in the skeet-store. Runs on the Hetzner k3s cluster. see [skeet-prune-pipeline.md](skeet-prune-pipeline.md)
- **skeet-refine** — applies more expensive LLM-based scoring (currently `gpt-4o` via OpenAI) to pruned candidates, assigning each a quality score, written back to the skeet-store. Runs on the Hetzner k3s cluster.
- **skeet-store** — stores found skeets and their scores in an S3-compatible store (Cloudflare R2, SSE-C encrypted), in tables managed as [LanceDB](https://lancedb.com) tables. Images are content-addressed by Bluesky blob CID. Structured as ports-and-adapters — see [skeet-store-architecture.md](skeet-store-architecture.md).
- **skeet-publish** — the single authority on *what* gets published. It computes ranked Redis lists per `(order, limit)` spec, combining automatic quality **Bands** with **manual appraisal overrides** and probing Bluesky to drop deleted posts. Lists are rebuilt on store table-version change and swapped atomically. Runs on the Hetzner k3s cluster.
- **skeet-feed** — Bluesky feed generator (AT Protocol feed skeleton) that serves the top-scored images as a custom feed, plus a server-rendered public image grid at `/` and a dynamic Open Graph preview image. Storeless and Fly-suspendable: reads the published Redis lists and lets Bluesky's CDN serve the images. Deployed on Fly.io.
- **skeet-appraise** — the admin/appraisal web UI (GitHub OAuth, allowlisted) for inspecting found skeets and manually banding them; those manual appraisals feed back into what skeet-publish publishes. Deployed on Fly.io.

All services emit OpenTelemetry traces + metrics to Grafana Cloud. See [remote-setup.md](remote-setup.md) for the Hetzner cluster and [versioning.md](versioning.md) for how production and per-worktree dev share the same R2/Redis stores.

## Constraints, Trade-offs and Technology Choices

- **Rust first:** all code should be in Rust where possible
- **ML:** use existing models or Rust libraries for face-detection and landmark identification
  - Non-Rust ML models are OK if really required
- **Burn:** use [Burn](https://burn.dev) ([GitHub](https://github.com/tracel-ai/burn)) for running ML models
- **Sampling:** processing at line-speed may not be possible — sampling is fine. Simple parts (e.g. checking if a message contains an image) should be inline with receiving a message.
- **Interactive UI:** prefer [htmx](https://htmx.org) for interactivity in HTML views (server-rendered templates returning HTML fragments). Avoid client-side JS frameworks unless htmx is genuinely insufficient.
- **Storage — LanceDB on R2, not Iceberg:** the store (`skeet-store`) is [LanceDB](https://lancedb.com) over Cloudflare R2. **Iceberg was considered and rejected.** It's a workload *mismatch* — Bobby does point lookups by content-hash `image_id` and stores ~2 MB image blobs per row, which Lance's scalar indices + blob handling serve well; Iceberg/Parquet is tuned for large analytical scans with file-level pruning and has no scalar point index, so it would be a downgrade. Adopting it is also a data-format migration, not an API swap. The one genuine draw — an open multi-engine catalog — is reachable via Lance Namespace without leaving Lance.

## Prune and Refine Pattern

The pipeline follows a **prune-and-refine** pattern:

1. **Prune** (`skeet-prune`): fast, approximate checks that discard the vast majority of candidates, run continuously over the firehose to reduce the stream to a sub-1% hit rate. Biased towards recall: a small percentage of false positives are acceptable because they will be caught in the refine stage. It's built as a staged stream-processing pipeline — see [skeet-prune-pipeline.md](skeet-prune-pipeline.md) for the stages and the checks each applies.

2. **Refine** (`skeet-refine`): expensive, precise scoring applied only to the candidates that survive pruning. Uses an LLM to evaluate how well each image matches the target intent (selfie with a recognizable landmark). Produces a score between 0.0 and 1.0.

Configuration for each stage lives in `config/`:
- `config/prune.toml` — thresholds for face area, skin percentages
- `config/refine.toml` — LLM provider, model name, and scoring prompt

Both configs produce a `ModelVersion` (a short hash of their contents) used to track which version of the config was active when an image was processed or scored.

## Testing

- **Blocklist as test fixture:** the `blocklist/` directory captures real examples of skeets that should have been blocked (adult content, `!no-unauthenticated` authors, etc.). Each entry has a `getPostThread` JSON snapshot. The blocklist is *not* used in the live pipeline — it exists purely to drive integration tests that verify the metadata filtering logic works correctly.
