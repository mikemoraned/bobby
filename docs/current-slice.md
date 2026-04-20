# Current Slice: Slice 16 — make costs visible and reduce them

### Target

I'd like to end up with a monthly cost profile which is roughly the following, ordered by intended dominant costs:
1. prune + live-refine: fixed monthly cost of the hetzner cluster running them
2. live-refine: there will be a variable some number of image candidates each month, but I'd like a per-day upper-bound on spend on LLM calls, which turns into effectively a fixed cost per month
3. feed running on fly.io: small cost per call from blusky feed
4. admin/appraising: small ad-hoc cost as I appraise images on fly.io

However, what I actually have, as of 19th Apr is:
1. Significant R2 costs, coming from Class A and B operations which go above the free allowance; this is easily $100's per month if left unchecked
2. live-refine LLM costs: I've been manually topping this up by $5 a day, which easily get eaten-up; this may lessen once the effect of the more tight text-detection based pruning kicks in
3. prune + live-refine: hetzner cluster running code: €10 or approx £8.7 on hetzner cluster
4. feed + admin/appraising running on fly.io: $1 or approx £0.74 per month

### Tasks

#### Get visibility on R2 usage

* [ ] implement an `object_store` wrapper for each lancedb `SkeetStore` user which logs a running metric every time a particular S3 API operation is used
    * I've registered for grafana cloud, so can use that instead of honeycomb, which may be easier to use

#### Idea: Switch to notification-listening for live-refine

* [ ] rather than polling the remote store for recently-updated images that have been pruned, the `pruner` and `live-refine` clis can communicate via a notification queue that says when an image candidate has been found.

#### Idea: put in place some sort of caching of Lancedb R2 lookups

* [ ] ...

#### Idea: run LLM models in batch mode

* [ ] ...

#### Idea: run a local model inside k8s cluster (via ollama)

* [ ] ...
