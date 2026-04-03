# Current Slice: Slice 9 — "Bobby Dev" Custom Feed in Bluesky

### Target

* a "Bobby Dev" [Custom Feed](https://docs.bsky.app/docs/starter-templates/custom-feeds) in Bluesky which I can use for live dev testing
    * this should surface the top N scored skeets, ordered by score, where score threshold > T, and have been in last H hours
        * initially, N = 10, T = 0.5, H = 48

### Tasks

* [x] refactors / cleanups
    * [x] rename `skeet-feed` to `skeet-inspect` capturing it's role of allowing inspection of what's been found. It doesn't need to be the actually exposed feed, so rename so I can use that name again for the main feed.
        * rename associated `Justfile` rules, and clis as well.
    * [x] `skeet-inspect` changes:
        * [x] rename `latest` -> `pruned` and `best` -> `refined` pages, but keep current functionality
        * [x] simplify `pruned` and `refined` to show the same page content format and entries, but keep current functionality
            * the only difference in format is that the `Score` column on `pruned` is allowed to just show `None` for rows that haven't been scored yet

* [x] `skeet-prune` changes:
    * I suspect the text-detection is not adding much benefit really
        * [x] before we remove it, let's add some analysis:
            * [x] add a `RejectionCategory` to each `Rejection` enum which is one of Face, Text, Metadata
            * [x] update live firehose pruner so that, alongside stats of how often each `Rejection` reason is used it also outputs:
                * [x] how often `RejectionCategory` is used (in raw numbers and %-ages)
                * [x] how often each `RejectionCategory` was the sole reason for rejection e.g. how often `RejectionCategory::Text` was the sole reason for a Skeet being pruned
        * ok, from the stats (see below), it looks like `Text` is only the sole rejection category 1% of the time. So, we can afford to get rid of it entirely (we can always bring it back later if needed)
        ```
        2026-03-29T23:04:12.963692Z  INFO skeet_prune::status: skeets: 17426 (1.9/s) | images: 17274 | saved: 33 (0.2%) | rejected: 21425 (BlockedByMetadata: 4198 [18%], FaceNotInAcceptedZone: 213 [1%], FaceTooLarge: 36 [0%], FaceTooSmall: 1691 [7%], TooFewFrontalFaces: 12491 [54%], TooLittleFaceSkin: 603 [3%], TooManyFaces: 2092 [9%], TooMuchSkinOutsideFace: 917 [4%], TooMuchText: 929 [4%]) | categories: Face: 17072 [80%] (sole: 16298 [76%]), Text: 929 [4%] (sole: 155 [1%]), Metadata: 4198 [20%] (sole: 4198 [20%])
        ```
            * [x] we should remove all associated crates / enums / model config etc. suggested strategy is to:
                * [x] remove downloaded models related to text, and the Justfile rules for them
                * [x] remove the `text-detection` crate from the workspace and fix all related compile errors
                * [x] do a pass to ensure we don't mention it anywhere anymore

* [x] a new `skeet-feed` which is just for the Bluesky Custom Feed:
    * this will run on fly.io as a new `bobby-staging.houseofmoran.io` app (called `bobby-staging`)
        * on fly.io this will be called `https://bobby-staging.fly.dev` and I will point `bobby-staging.houseofmoran.io` at that
    * should be built using a `Dockerfile` called `Dockerfile.skeet-feed` and follow current best practices for building and deploying Rust apps via Docker
    * this should be another `cot.rs` website
    * it should connect to the remote R2 store
    * see Bluesky docs:
        * https://docs.bsky.app/docs/api/app-bsky-feed-get-feed-skeleton
        * https://docs.bsky.app/docs/starter-templates/custom-feeds
    * [x] write a helper that takes the `bobby.env` and syncs it with fly.io secrets
    * [x] this should use a deploy approach which is similar, but not same, as https://github.com/mikemoraned/fosdem/blob/main/Justfile i.e.
        * deploy_staging
        * test_webapp (which uses `cot.rs` integ tests)
        * test_staging (which runs same `cot.rs` integ tests against staging, after `deploy_staging` completes)
    * [x] write a helper rust cli that allows a Custom Feed be registered with bluesky
        * see `scripts/publishFeedGen.ts` mentioned in [docs](https://docs.bsky.app/docs/starter-templates/custom-feeds) and https://crates.io/crates/skyfeed for inspiration
    * [x] register feed


* [ ] `skeet-refine` changes:
    * [x] update refine step so that when it finds images needing scored, it orders them by most recently discovered (so that more recently added images get assigned a score first)
    * [x] update live-refine so that it runs once every minute (as now) but once it finds a list of images to score, it only runs for one minute before looping back to check for more recently-added images that don't have a score. 
        * the intent is that it is always trying to score the latest images, but still allows itself to score older images when it's done the latest
        * to find images that don't have a score, it may be more efficient to add an index; see lancedb.com docs for advice
    * ...