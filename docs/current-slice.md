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

* [ ] for each of the smells break out possible causes and options for fixes. this may also include adding more visibility through opentelemetry or other tracing
* [ ] similarly, for each of the ideas
* [ ] similarly, for each of the benchmarks
* [ ] ...
