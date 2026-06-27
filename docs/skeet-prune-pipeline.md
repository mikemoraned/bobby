# `skeet-prune` pipeline architecture

`skeet-prune` is a **staged stream-processing pipeline**. A skeet enters at the
firehose, is narrowed by a series of filtering stages, and what survives is saved
along with metadata recording what happened to everything that flowed through.

## Stages

```
ingest ──▶ filter ──▶ … ──▶ filter ──▶ save
```

The first stage fetches data from the firehose. Each subsequent stage filters by
a different criterion, dropping what it rejects and passing the rest on. The
final stage persists what made it through, together with the metadata about what
was done.

Stages run concurrently and communicate over bounded channels, so a slow stage
applies backpressure upstream rather than letting work pile up unboundedly. A
stage that is the throughput bottleneck can be widened into a pool of workers
sharing its input without changing the shape of the pipeline.

Concretely, `skeet-prune`'s stages are:

1. **Ingest** — read image-bearing posts as they arrive on the Bluesky firehose.
2. **Metadata filter** — drop posts whose thread metadata excludes them (e.g.
   adult-content labels, `!no-unauthenticated` authors).
3. **Image filter** — download each image and run cheap detectors — face
   detection, skin detection, and optional text detection — keeping only images
   that plausibly match the target (a selfie with a recognizable landmark).
4. **Save** — persist the survivors to the store, recording the run's tallies.

The checks deliberately favour recall over precision: cheap and approximate
here, with the expensive precise judgement left to `skeet-refine` downstream. The
image filter is the throughput-bound stage and runs as a worker pool.

## Messages carry work *and* metadata

Each stage emits a tuple message to the next: the **work** for the next stage to
consider, paired with **metadata** recording what has happened so far.

The work half narrows as it flows — a candidate to examine becomes the items
worth keeping. When a stage decides there is nothing left to do, the work half is
empty, but the message still flows, because it still carries metadata.

## Each stage accumulates the metadata

The metadata is a running tally, and the invariant is:

> Each stage forwards metadata that is the accumulation of everything that
> happened upstream, plus whatever happened in that stage.

So the stage that makes a decision is the stage that records it — no stage passes
a raw "this was rejected" marker downstream just so something else can count it.
This is what lets the work half shrink to nothing while the metadata still
carries the full story, and it lets the final stage simply merge each message's
metadata into a grand total rather than re-deriving anything.

The tally is a **monoid**: there is an empty value and an associative combine, so
"accumulate upstream plus mine" is one operation and the order of merging never
matters.

### Two deliberate exceptions

- **What was *saved* is recorded at the final stage**, not accumulated up the
  pipeline. Whether an item is actually persisted depends on storage state (it
  may already exist), so only the stage that writes it knows.
- **Stage throughput and queue depth are measured out-of-band**, not folded into
  the per-message metadata. They answer a different question — *is this stage the
  bottleneck?* — about pipeline health rather than about what was observed in the
  stream, so they are kept as a separate, independent measurement.

## Shutdown

All stages share one cancellation signal. If any stage's downstream goes away,
that signal is tripped and every other stage unwinds through the same seam,
rather than each stage detecting a closed channel its own way.

## Why this shape

- **One counting path.** Counting rides the data plane as a monoid merged at the
  end. Adding a new rejection reason or a new stage means contributing to the
  tally — there is no separate counting protocol to keep in sync with the work.
- **Minimal work payloads.** Because counting travels in the metadata half, the
  work half is exactly what the next stage needs and nothing more.
- **Visible topology.** The ordered stages make "this is a pipeline" the first
  thing the code's structure tells you.
