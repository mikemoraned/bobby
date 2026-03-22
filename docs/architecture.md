# Architecture

## Background

This project recreates [the original Twitter-based selfie finder](https://github.com/mikemoraned/selfies) ([blog post](https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/)) using Bluesky instead of Twitter and modern technologies.

The original project scanned Twitter's firehose, applied face detection (via OpenIMAJ), and looked for selfie-like compositions (face in foreground border, landmark in background). It had a ~0.1% hit rate and encountered challenges like porn, screenshots, and false positives from inanimate objects resembling faces.

## Target Architecture

- **skeet-finder** — continuously listens to the Bluesky firehose and detects skeets containing images showing the content we want, then stores them in the skeet-store
- **skeet-store** — stores found skeets in an S3-compatible store, in tables, managed as [LanceDB](https://lancedb.com) tables
- **skeet-feed** — HTTP service that reads from the store and surfaces all found skeets as a Bluesky Feed

## Constraints, Trade-offs and Technology Choices

- **Rust first:** all code should be in Rust where possible
  - Acceptable to use non-Rust for getting Bluesky firehose data including images; once an image is fetched, everything else must be Rust
  - Non-Rust ML models are OK
- **ML:** use existing models or Rust libraries for face-detection and landmark identification
- **Burn:** use [Burn](https://burn.dev) ([GitHub](https://github.com/tracel-ai/burn)) for running ML models
- **Sampling:** processing at line-speed may not be possible — sampling is fine. Simple parts (e.g. checking if a message contains an image) should be inline with receiving a message.

## Testing

- **Blocklist as test fixture:** the `blocklist/` directory captures real examples of skeets that should have been blocked (adult content, `!no-unauthenticated` authors, etc.). Each entry has a `getPostThread` JSON snapshot. The blocklist is *not* used in the live pipeline — it exists purely to drive integration tests that verify the metadata filtering logic works correctly.
