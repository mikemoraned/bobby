# Bobby

Find selfies people take of themselves with physical landmarks (famous buildings, monuments, places like the Eiffel Tower) — using Bluesky's firehose and modern ML models in Rust. Recreates [the original Twitter-based project](https://github.com/mikemoraned/selfies) ([blog post](https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/)).

The result is live: the public feed at [bobby.houseofmoran.io](https://bobby.houseofmoran.io) and as a subscribable [custom feed on Bluesky](https://bsky.app/profile/did:plc:cjvdzmk4iapi5p5orrasehxp/feed/bobby).

## What version 1.0 is

A continuously-running pipeline that watches the whole Bluesky firehose, filters it down to landmark selfies, scores them for quality, and serves the best as a public feed. It follows a **prune-and-refine** shape — cheap, recall-biased ML filtering first (`skeet-prune`), then expensive, precise LLM scoring only on the survivors (`skeet-refine`) — with a publisher deciding what to show and two web apps serving it.

It's Rust throughout: ONNX/OCR models run locally in the pruner, LLM scoring via OpenAI, server-rendered HTML with [htmx](https://htmx.org). The pruner/refiner/publisher run on a single-node k3s cluster on Hetzner; the web apps on Fly.io; data lives in shared LanceDB-on-R2 and Redis stores; everything is observable via OpenTelemetry to Grafana Cloud.

See [docs/architecture.md](docs/architecture.md) for the full service breakdown and [docs/completed-slices.md](docs/completed-slices.md) for the slice-by-slice history.

## Working on it

See [CLAUDE.md](CLAUDE.md) for prerequisites, key commands, and conventions.
