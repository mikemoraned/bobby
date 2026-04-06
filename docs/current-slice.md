# Current Slice: Slice 12 — Optimisations of pruning, refining, and feeding

### Smells to be investigated / addressed

1. looking at cpu and network usage in hetzner:
    * we can see it going up and down:
        * cpu goes from 100% to 200% over a period of about 30 secs
        * network bandwidth goes from 0 to 80Mbps over similar time-scale
    * this feels a bit like a bottleneck being encountered periodically like a buffer being filled and meeting a limit
    * we also log every 30 seconds, so it's possible this is somehow blocking things (as it sits at end of pipeline)
2. looking at traces for operations like `get_by_id` these take about 10 seconds and make multiple `read_fragment` calls in `lance::io::exec::filtered_read`
    * this kinda looks like a table scan; couldn't this be an index lookup instead?
    * also, could we add a dump of the plan to the trace (in `plan_scan`) so we can see what it is doing?
    * I see similar traces for things like `exists`
3. the read of the feed itself takes several seconds
    * this could be caused by the same or similar problems as 2 i.e. long scans

### Optimisation ideas

* live-refine: when we find a new set of images to score:
    * we can dispatch multiple calls in parallel to openai as we're largely waiting on them to respond (it's i/o bound)
    * once we have some scores, we can batch-save them to lancedb (lancedb recommends batch-saving to reduce fragmentation)

### Benchmarking

* we should be able to measure the maximum possible speed the pruner can run by taking the jetstream stage only and running that on it's own
    * we should be able to run a minimal cli instance and associated k8s deployment which just runs this step and summarises speeds
    * probably should just run for 5 minutes and collate some statistics before dumping them out at the end

### Tasks

#### Smells (investigate first — #2 likely explains #3 and may contribute to #1)

* [ ] #2: add query plan logging (`explain_plan`) to `get_by_id` and `exists`; verify `.only_if()` actually uses the scalar index
* [ ] #2: log table row counts and fragment counts at startup to assess fragmentation
* [ ] #3: log row counts in `list_scored_summaries_by_score` to see if the full-table reads are the bottleneck (likely same root cause as #2)
* [ ] #1: add channel depth and per-stage throughput logging to the pruner pipeline
* [ ] #1: make the 30s status logging interval configurable; check if it blocks the save stage

#### Benchmarking

* [x] create a minimal `bench-firehose` binary that runs the jetstream stage only for 5 mins and reports messages/sec stats
* [x] add `just bench-firehose` target and k8s deployment for running on Hetzner

Results:
* locally (running on laptop):
```
2026-04-06T00:28:50.798585Z  INFO bench_firehose: === firehose benchmark results ===
2026-04-06T00:28:50.798684Z  INFO bench_firehose: totals elapsed_secs=300.0 total_candidates=1643 total_images=2139
2026-04-06T00:28:50.798704Z  INFO bench_firehose: throughput candidates_per_sec=5.5 images_per_sec=7.1
```
* hetzner cluster (running shared with everything else):
```
2026-04-06T01:32:45.906157+0100  INFO bench_firehose: === firehose benchmark results ===
2026-04-06T01:32:45.906180+0100  INFO bench_firehose: totals elapsed_secs=300.0 total_candidates=1802 total_images=2358
2026-04-06T01:32:45.906192+0100  INFO bench_firehose: throughput candidates_per_sec=6.0 images_per_sec=7.9
2026-04-06T01:37:48.232015+0100  INFO bench_firehose: === firehose benchmark results ===
2026-04-06T01:37:48.232047+0100  INFO bench_firehose: totals elapsed_secs=300.0 total_candidates=1677 total_images=2249
2026-04-06T01:37:48.232058+0100  INFO bench_firehose: throughput candidates_per_sec=5.6 images_per_sec=7.5
```

Conclusions:
* firehose delivers ~5.5–6.0 candidates/sec (~7–8 images/sec) consistently across laptop and Hetzner
* since results are nearly identical regardless of location, the bottleneck is the firehose itself (rate of image-bearing posts on Bluesky), not our compute or network
* ~1.3 images per candidate on average
* this sets the input ceiling for the pruner pipeline: each candidate must be processed in under ~170ms on average to keep up

#### Optimisations (act on information from above first)

* [ ] live-refine: dispatch OpenAI calls in parallel (currently sequential)
* [ ] live-refine: batch-save scores to lancedb to reduce fragmentation
