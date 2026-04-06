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
* [x] extend the benchmark to have an image fetch stage i.e.
    * run for 5 minutes, collecting candidates and images (stay as-is)
      * however extend this stage so that it remembers (but doesn't fetch the images)
    * add a new stage that goes through these images one at a time, grouped by status code, measures:
      * latency of download per image
      * latency per byte

Results:
* locally (running on laptop):
```
2026-04-06T01:25:06.349145Z  INFO bench_firehose: === phase 1: firehose results ===
2026-04-06T01:25:06.349173Z  INFO bench_firehose: totals elapsed_secs=300.0 total_events=11123 total_candidates=1571 total_images=2034 candidate_pct=14.1%
2026-04-06T01:25:06.349186Z  INFO bench_firehose: throughput events_per_sec=37.1 candidates_per_sec=5.2 images_per_sec=6.8
...
2026-04-06T01:27:38.589308Z  INFO bench_firehose: === phase 2: image fetch results ===
2026-04-06T01:27:38.589371Z  INFO bench_firehose: by status status="200" count=2034 avg_latency_ms=48.2 min_latency_ms=24 max_latency_ms=3975 avg_bytes=90581 total_bytes=184240877
```
* hetzner cluster (running shared with everything else):
```
2026-04-06T01:27:08.123435Z  INFO bench_firehose: === phase 1: firehose results ===
2026-04-06T01:27:08.123469Z  INFO bench_firehose: totals elapsed_secs=300.0 total_events=10979 total_candidates=1594 total_images=2055 candidate_pct=14.5%
2026-04-06T01:27:08.123487Z  INFO bench_firehose: throughput events_per_sec=36.6 candidates_per_sec=5.3 images_per_sec=6.8
...
2026-04-06T01:31:11.238232Z  INFO bench_firehose: === phase 2: image fetch results ===
2026-04-06T01:31:11.238476Z  INFO bench_firehose: by status status="200" count=2055 avg_latency_ms=100.7 min_latency_ms=6 max_latency_ms=1427 avg_bytes=92362 total_bytes=189804780
```

Conclusions:
* jetstream delivers ~37 posts/sec (filtered to `app.bsky.feed.post` at connection level)
* ~15% of posts have images, giving ~5–6 candidates/sec (~7 images/sec, ~1.3 images per candidate)
* results are nearly identical across laptop and Hetzner, confirming the rate is set by Bluesky's post volume, not our compute or network
* this sets the input ceiling for the pruner pipeline: at ~6 candidates/sec, each candidate must be processed in under 1/6s ≈ 170ms on average to keep up
* image fetches: all 200s (no errors), ~90KB average per image
    * laptop: avg 48ms, min 24ms, max 4s — fetched ~2k images in ~2.5 mins
    * hetzner: avg 101ms, min 6ms, max 1.4s — fetched ~2k images in ~4 mins
    * surprisingly, laptop has lower avg latency than Hetzner (possibly CDN edge proximity)
    * at ~50–100ms per image sequentially, fetching 7 images/sec would require parallelism (sequential can only do 10–20/sec)

Optimisations to consider:
* pruner image stage currently downloads + classifies images sequentially per candidate — at 100ms/image on Hetzner, a 2-image candidate takes ~200ms, already over the 170ms budget before classification even runs
* could overlap image downloads within a candidate (fetch all images concurrently, classify as they arrive)
* could pipeline across candidates: start fetching the next candidate's images while classifying the current one
* image classification (face/skin detection) is CPU-bound and probably fast compared to the network fetch, so the fetch is the bottleneck to target first

#### Optimisations (act on information from above first)

* [ ] live-refine: dispatch OpenAI calls in parallel (currently sequential)
* [ ] live-refine: batch-save scores to lancedb to reduce fragmentation
