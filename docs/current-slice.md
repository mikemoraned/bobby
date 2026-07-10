# Current Slice: 1.0 final tweaks and improvements

* [x] mark all crates to be at version 1.0
* [ ] features:
  * a new `quality,recency-12w` mode used by default on main feed site (bobby.houseofmoran.io)
    * [ ] add a new `quality,recency-{limit}` publishing mode where we can combine sorting by quality and recency such that we first sort by quality into Band buckets and then within those buckets sort by recency. We should think of `quality,recency` as a new standalone Order::QualityRecency as we don't want to support the other way round i.e. we *don't want* `recency,quality`. No need to use this mode in this step i.e. we add it as an ability but don't use it yet
      * [ ] add `Order::QualityRecency`; `Display` → `quality,recency`, `FromStr` parses it (round-trip tests). The `{order}-{limit}` split stays unambiguous — the token has a comma, not a dash; the list name is `v3-quality,recency-12w`
      * [ ] add a `QualityRecencyRank`: effective Band bucket (best first — same effective-band computation, incl. manual appraisal overrides, as `Quality`), then recency (newest `original_at`) within the bucket, then normalized score, then `image_id`/`skeet_id` as the final deterministic tie-break
      * [ ] wire it into `published_for_spec`'s `match order`
      * [ ] extend the exhaustive `dropdown_key` match in `available_feeds.rs`, ranking `quality,recency` **first** (before `quality`, then `recency`)
      * [ ] add the variant to the fallback `prop_oneof!` generator
      * [ ] unit tests mirroring the Quality suite but asserting recency as the intra-band key: band-beats-recency, within-band newest-first, score breaks equal-timestamp ties, manual-band-override reorders, below-threshold hidden, deterministic ties
      * [ ] confirm `PublishedList` name round-trips for `(QualityRecency, 12w)`; run `just clippy` + `just mutants-on-diff-no-docker`
      * out of scope here (later bullets): adding it to any `--publish` config, making it a default, or deploying
   * [ ] start publishing `quality,recency-12w` and make this the default used on main feed site:
    * [ ] update code / config
    * [ ] deploy publisher
    * [ ] deploy to staging (bobby-staging.houseofmoran.io) and verify working as expected
    * [ ] deploy to bobby.houseofmoran.io and similarly verify
    * [ ] stop publishing any `quality-{limit}` lists anymore as nothing should need it, but keep the Order::Quality capability; deploy publisher and manually remove unneeded lists
* [ ] checks docs / skills
  * [ ] check CLAUDE.md and similar include links to all relevant docs e.g. anything architectural in docs
  * [ ] do a pass over all rust code, checking if it follows guidelines of `rust.md`
  * [ ] do a pass over `rust.md` skill to check that is logical, internally consistent, and minimal with no duplication.
    * [ ] do a comparison against similar advice on web, or any relevant advice that could be included by reference.
    * [ ] do another pass over all rust code, checking if it follows guidelines of the new `rust.md` (want to see if any advice becomes contradictory)
* [ ] expand README.md to cover:
  * a short summary of what this is (if possible let's dedupe this from CLAUDE.md and move it to README.md)
  * a short summary what version 1.0 contains in terms of features / approach / capabilities, based on contents of completed-slices.md. This should be high-level and not a lot of detail; think of what you would do if you had 5 mins to explain what this is and how it work to a generally knowledge-able senior engineer co-worker.
