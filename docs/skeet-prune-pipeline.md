# `skeet-prune` pipeline architecture

`skeet-prune` is a **staged stream-processing pipeline**. A skeet enters at the
firehose, is narrowed by a series of filtering stages, and what survives is saved
along with metadata recording what happened to everything that flowed through.

## Stages

```
ingest ──▶ filter ──▶ … ──▶ filter ──▶ save ──▶ stats
```

The first stage fetches data from the firehose. Each subsequent filter stage
filters by a different criterion, dropping what it rejects and passing the rest
on. The save stage persists what made it through; the final stage merges the
metadata about what was done into a running tally and reports it.

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
4. **Save** — persist the survivors to the store, folding the resulting `saved`
   count into the metadata it forwards on.
5. **Content statistics** — merge each message's metadata into one running
   total and periodically log and emit the summary. It does no storage work.

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
metadata into a grand total rather than re-deriving anything. The save stage is
no exception: whether an item is actually persisted depends on storage state (it
may already exist), so the save stage is the one that knows, and it folds its
`saved` decision into the metadata it forwards just like every other stage.

The tally is a **monoid**: there is an empty value and an associative combine, so
"accumulate upstream plus mine" is one operation and the order of merging never
matters.

### One deliberate exception

- **Stage throughput and queue depth are measured out-of-band**, not folded into
  the per-message metadata. They answer a different question — *is this stage the
  bottleneck?* — about pipeline health rather than about what was observed in the
  stream, so they are kept as a separate, independent measurement.

## Shutdown

Shutdown has two shapes, distinguished by intent.

**Reactive abort.** All stages share one *abort* cancellation signal. If any
stage's downstream goes away, that signal is tripped and every other stage
unwinds at once through the same seam, rather than each stage detecting a closed
channel its own way. In-flight and buffered work is dropped — there is nowhere
for it to go once the pipeline is torn down.

**Deliberate drain.** A process-termination signal (SIGTERM on a k8s redeploy,
SIGINT on Ctrl-C) trips a *separate* drain signal that stops only the **source**
— the firehose stops pulling new events and drops its output sender. Because the
depth monitors hold only *weak* handles onto the channels, that dropped sender
closes the firehose channel, and closure cascades downstream: each stage drains
the items already buffered ahead of it, finishes, and drops its own output
sender, closing the next channel in turn. Everything already in the pipeline
finishes into the store, which is safe to do precisely because the sink is
content-hash idempotent. This narrows — but does not eliminate — restart loss:
the in-memory resume cursor still resumes at live-tail after a restart, so the
gap while the process is down is still lost; only what was already in the
pipeline at signal time is saved.

The stages are supervised (a `JoinSet`) and awaited, so shutdown waits for the
whole pipeline to drain rather than only for the sink. Two guards bound the
wait: a drain deadline (kept under the k8s grace period) forces exit if a stage
gets stuck, and a second termination signal escalates to a reactive abort.

## Why this shape

- **One counting path.** Counting rides the data plane as a monoid merged at the
  end. Adding a new rejection reason or a new stage means contributing to the
  tally — there is no separate counting protocol to keep in sync with the work.
- **Minimal work payloads.** Because counting travels in the metadata half, the
  work half is exactly what the next stage needs and nothing more.
- **Visible topology.** The ordered stages make "this is a pipeline" the first
  thing the code's structure tells you.
