# Bobby

Find selfies people take of themselves with physical landmarks (famous buildings, monuments, places like the Eiffel Tower) — using Bluesky's firehose and modern ML models in Rust. Recreates [the original Twitter-based project](https://github.com/mikemoraned/selfies) ([blog post](https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/)).

The result is live: the public feed at [bobby.houseofmoran.io](https://bobby.houseofmoran.io) and as a subscribable [custom feed on Bluesky](https://bsky.app/profile/did:plc:cjvdzmk4iapi5p5orrasehxp/feed/bobby).

## What version 1.0 is

A continuously-running pipeline that watches the whole Bluesky firehose, filters it down to landmark selfies, scores them for quality, and serves the best as a public feed. It follows a **prune-and-refine** shape — cheap, recall-biased filtering first; expensive, precise scoring only on the survivors — split across a handful of small Rust services (see [docs/architecture.md](docs/architecture.md)):

- **`skeet-prune`** — listens to the firehose (via Jetstream) and runs fast, approximate ML checks to throw away the ~99.8% of images that can't be a match: **face detection** (YuNet ONNX) for a face in a corner/edge zone, **skin detection** to reject adult content, and **text detection** (OCR) to reject screenshots/memes. It's a staged, back-pressured stream pipeline that drains cleanly on shutdown and resumes losslessly across firehose reconnects. Surviving images are saved to the store.
- **`skeet-refine`** — applies expensive, precise **LLM scoring** (currently `gpt-4o`) to pruned candidates, producing a 0.0–1.0 quality score. Configs are content-hashed into a `ModelVersion` so every score records which model/prompt produced it.
- **`skeet-store`** — the storage layer: **[LanceDB](https://lancedb.com) tables on Cloudflare R2** (S3-compatible, SSE-C encrypted at rest), images content-addressed by Bluesky blob CID. Structured as ports-and-adapters so consumers depend only on the narrow trait they need (see [docs/skeet-store-architecture.md](docs/skeet-store-architecture.md)).
- **`skeet-publish`** — the single authority on *what* gets published: it computes ranked Redis lists per `(order, limit)` spec, combining automatic quality **Bands** with **manual appraisal overrides**, then probes Bluesky to drop deleted posts. The default ordering is `quality,recency` (best band first, newest within a band).
- **`skeet-feed`** — a storeless, suspendable web app serving the **Bluesky custom-feed skeleton** plus a server-rendered masonry image grid at `/`, with a dynamic Open Graph preview image. Reads the published Redis lists; images are served straight from Bluesky's CDN.
- **`skeet-appraise`** — the admin/appraisal UI (GitHub OAuth, allowlisted) for manually banding skeets and images, feeding the overrides `skeet-publish` respects.

**Approach & stack:** Rust everywhere; server-rendered HTML with [htmx](https://htmx.org) rather than a JS framework; ONNX/OCR models run locally in the pruner, LLM scoring via OpenAI. The pruner and refiner run on a single-node **k3s cluster on Hetzner**; the web apps run on **Fly.io**. Everything is observable through OpenTelemetry traces + metrics to **Grafana Cloud**, and the whole system runs at roughly £27/month. Production and per-worktree dev coexist on the *shared* R2/Redis stores via a versioning convention rather than duplicated infra (see [docs/versioning.md](docs/versioning.md)).

For the full slice-by-slice history of how it got here, see [docs/completed-slices.md](docs/completed-slices.md).

## Working on it

See [CLAUDE.md](CLAUDE.md) for prerequisites, key commands, and conventions.
