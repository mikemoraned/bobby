# Current Slice: drain the pipeline on SIGTERM (close the restart in-flight loss)

### Target

The firehose slice made *reconnect* gaps lossless (cursor) but did nothing for *restart* loss. Shutdown today is purely **reactive**: stages unwind only when a channel closes (`docs/skeet-prune-pipeline.md` "Shutdown"). On a k8s redeploy (SIGTERM → SIGKILL after the grace period) the pruner is hard-killed with up to ~16+16+100 buffered items plus in-flight download/classify work across the three channels — all silently lost.

This is the other half of restart-loss the firehose slice deliberately deferred (the cursor's in-memory-only choice "accepts the restart gap as lost"; this slice narrows that gap to "drain what's already in the pipeline").

### Decisions / groundwork

- **The shared `CancellationToken` already exists** — the firehose slice introduced it as the closed-downstream seam (`pipeline.rs` `forward`/`recv`). This slice adds the *deliberate* trip, not a from-scratch shutdown mechanism.
- The shape: a SIGTERM handler trips the token to stop the **source** (firehose recv loop), then the stages are supervised (e.g. a `JoinSet`) and awaited so the bounded channels **drain into the idempotent store** before exit. Draining is safe precisely because the sink is content-hash idempotent.
- Compose with the cursor, don't compete: the cursor handles reconnect; this handles graceful restart. Persisting the cursor across restarts stays out of scope (a separate, larger choice).

### Tasks

* [ ] SIGTERM/SIGINT handler that trips the shared `CancellationToken` to stop the firehose source.
* [ ] Supervise the stages (`JoinSet` or equivalent) so `main` awaits their completion rather than only awaiting the sink; on shutdown, let the channels drain into the store before exit, bounded by a drain deadline shorter than the k8s grace period.
* [ ] Update `docs/skeet-prune-pipeline.md` "Shutdown" to describe deliberate drain alongside the reactive close.
* [ ] Verify `just clippy` + `just test-no-docker`. A deterministic loop test is awkward (signal/time-bound); a live SIGTERM smoke-check (observe the channels drain, no lost in-flight items) is the human/CI step.
