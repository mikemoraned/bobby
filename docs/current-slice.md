# Current Slice: 1.0 final tweaks and improvements

* [x] mark all crates to be at version 1.0
* [ ] features:
  * a new `quality,recency-12w` mode used by default on main feed site (bobby.houseofmoran.io)
    * [x] add a new `quality,recency-{limit}` publishing mode where we can combine sorting by quality and recency such that we first sort by quality into Band buckets and then within those buckets sort by recency. We should think of `quality,recency` as a new standalone Order::QualityRecency as we don't want to support the other way round i.e. we *don't want* `recency,quality`. No need to use this mode in this step i.e. we add it as an ability but don't use it yet
      * [x] add `Order::QualityRecency`; `Display` → `quality,recency`, `FromStr` parses it (round-trip tests). The `{order}-{limit}` split stays unambiguous — the token has a comma, not a dash; the list name is `v3-quality,recency-12w`
      * [x] add a `QualityRecencyRank`: effective Band bucket (best first — same effective-band computation, incl. manual appraisal overrides, as `Quality`), then recency (newest `original_at`) within the bucket, then normalized score, then `image_id`/`skeet_id` as the final deterministic tie-break
      * [x] wire it into `published_for_spec`'s `match order`
      * [x] extend the exhaustive `dropdown_key` match in `available_feeds.rs`, ranking `quality,recency` **first** (before `quality`, then `recency`)
      * [x] add the variant to the fallback `prop_oneof!` generator
      * [x] unit tests mirroring the Quality suite but asserting recency as the intra-band key: band-beats-recency, within-band newest-first, score breaks equal-timestamp ties, manual-band-override reorders, below-threshold hidden, deterministic ties
      * [x] confirm `PublishedList` name round-trips for `(QualityRecency, 12w)`; run `just clippy` + `just mutants-on-diff-no-docker`
      * out of scope here (later bullets): adding it to any `--publish` config, making it a default, or deploying
    * [x] start publishing `quality,recency-12w` and make this the default used on main feed site:
        * [x] update code / config
        * note: defaults split by consumer — Bluesky feed + appraiser default to `quality,recency-48h`, website image grid to `quality,recency-12w`. Publisher publishes `quality,recency-{48h,7d,4w,12w,1y}` (all current quality windows + 12w) so both fallback chains have coverage. `quality-{limit}` lists still published for now — removed only in the final sub-step after the deploys verify.
        * [x] deploy publisher
        * [x] deploy to staging feed (bobby-staging.houseofmoran.io) and appraiser and verify working as expected
        * [x] deploy to bobby.houseofmoran.io and appraiser and similarly verify
        * [x] stop publishing any `quality-{limit}` lists anymore as nothing should need it, but keep the Order::Quality capability; deploy publisher and manually remove unneeded lists
          * dropped `quality-{48h,7d,4w,1y}` from the publisher `--publish` set (k8s + justfiles); `recency-*` and `quality,recency-*` kept, `Order::Quality` code capability retained. Publisher deployed; orphaned `v3-quality-{48h,7d,4w,1y}` redis keys (+ `:statistics` / `:refreshed-at` companions) removed manually.
  * [ ] make the grid display in the feed show the top entries of the list further up the page. as it stands right now, we show the best entries first in a column that goes all the way to the bottom of the page before looping round to the top. We want to show the best content further up the page whilst still showing it as a grid, similar in style to what we have now.
* [ ] checks docs / skills
  * [x] check CLAUDE.md and similar include links to all relevant docs e.g. anything architectural in docs
  * [x] do a pass over all rust code, checking if it follows guidelines of `rust.md`, and fix them
    * pass done: codebase is in strong compliance (binary layout, no dead_code/macros, clippy clean, `expect`/`unwrap` all justified, comment hygiene, `-> bool` all genuine predicates). Two nits to fix:
    * [x] `crates/text-detection/src/lib.rs` is 419 lines (over the 300 soft limit) — ~185 code + ~233 inline tests. Split `TextDetectionResult`'s metrics (`character_count`/`text_area_pct`/`full_text`) and their tests into a `metrics` module
    * [x] `crates/bluesky/src/post_thread.rs:59` — replace the `let Some(..) = .. else { continue }` over `LABEL_PATHS` with `.iter().filter_map(...)` per the "avoid `continue`" rule
  * [x] do a pass over `rust.md` skill to check that is logical, internally consistent, and minimal with no duplication.
    * [x] do a comparison against similar advice on web, or any relevant advice that could be included by reference.
  * [ ] do another pass over all rust code, checking if it follows guidelines of the new `rust.md` (want to see if any advice becomes contradictory)
* [ ] expand README.md to cover:
  * a short summary of what this is (if possible let's dedupe this from CLAUDE.md and move it to README.md)
  * a short summary what version 1.0 contains in terms of features / approach / capabilities, based on contents of completed-slices.md. This should be high-level and not a lot of detail; think of what you would do if you had 5 mins to explain what this is and how it work to a generally knowledge-able senior engineer co-worker.
