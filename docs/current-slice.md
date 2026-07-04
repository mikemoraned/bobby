# Current Slice: drain the pipeline on SIGTERM (close the restart in-flight loss)

### Target

The firehose slice made *reconnect* gaps lossless (cursor) but did nothing for *restart* loss. Shutdown today is purely **reactive**: stages unwind only when a channel closes (`docs/skeet-prune-pipeline.md` "Shutdown"). On a k8s redeploy (SIGTERM → SIGKILL after the grace period) the pruner is hard-killed with up to ~16+16+100 buffered items plus in-flight download/classify work across the three channels — all silently lost.

This is the other half of restart-loss the firehose slice deliberately deferred (the cursor's in-memory-only choice "accepts the restart gap as lost"; this slice narrows that gap to "drain what's already in the pipeline").

### Decisions / groundwork

- **The shared `CancellationToken` already exists** — the firehose slice introduced it as the closed-downstream seam (`pipeline.rs` `forward`/`recv`). This slice adds the *deliberate* trip, not a from-scratch shutdown mechanism.
- The shape: a SIGTERM handler trips the token to stop the **source** (firehose recv loop), then the stages are supervised (e.g. a `JoinSet`) and awaited so the bounded channels **drain into the idempotent store** before exit. Draining is safe precisely because the sink is content-hash idempotent.
- Compose with the cursor, don't compete: the cursor handles reconnect; this handles graceful restart. Persisting the cursor across restarts stays out of scope (a separate, larger choice).

### Tasks

* [x] SIGTERM/SIGINT handler that trips the shared `CancellationToken` to stop the firehose source.
* [x] Supervise the stages (`JoinSet` or equivalent) so `main` awaits their completion rather than only awaiting the sink; on shutdown, let the channels drain into the store before exit, bounded by a drain deadline shorter than the k8s grace period.
  * Deviation from "trip the shared token": the shared token means *abort now* at every `recv`/`forward`, which drops buffered work — wrong for a drain. So it kept that role (renamed `abort`) and a **separate `drain` token** stops only the source; the channels then close-cascade. That cascade required `ChannelMonitors` to hold *weak* senders — strong clones there were pinning the channels open, which is why the token had been the only viable shutdown path. Second signal escalates drain → abort; deadline is `--drain-timeout-secs` (default 25).
* [x] Update `docs/skeet-prune-pipeline.md` "Shutdown" to describe deliberate drain alongside the reactive close.
* [x] Verify `just clippy` + `just test-no-docker` — both green (606 passed, 33 docker-skipped). The live SIGTERM smoke-check (observe the channels drain, no lost in-flight items) remains a by-hand step.
* [ ] `crates/skeet-prune/src/bin/pruner.rs` has grown large — the pipeline assembly (channel creation, per-stage spawns into the `JoinSet`, signal + drain wiring) could move into a `pipeline` submodule (e.g. `pipeline::assemble`/`run`), leaving the bin as arg-parsing + config + a single call.
