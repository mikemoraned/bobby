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

* [ ] `skeet-prune` changes:
    * I suspect the text-detection is not adding much benefit really
        * [x] before we remove it, let's add some analysis:
            * [x] add a `RejectionCategory` to each `Rejection` enum which is one of Face, Text, Metadata
            * [x] update live firehose pruner so that, alongside stats of how often each `Rejection` reason is used it also outputs:
                * [x] how often `RejectionCategory` is used (in raw numbers and %-ages)
                * [x] how often each `RejectionCategory` was the sole reason for rejection e.g. how often `RejectionCategory::Text` was the sole reason for a Skeet being pruned
            * [x] a replay analysis:
                * add a `replay` cli to `skeet-prune` which takes a local or remote store, and replays the images through the pruning pipeline
                    * this can effectively be seen as replacing the firehose input after it has fetched images
                    * we should re-use as much as possible from existing pipeline
        * [ ] if we remove it we should remove all associated crates / enums / model config etc

* [ ] a new `skeet-feed` which is just for the Bluesky Custom Feed:
    * this will run on fly.io as a new `bobby-staging.houseofmoran.io` app (called `bobby-staging`)
        * on fly.io this will be called `https://bobby-staging.fly.dev` and I will point `bobby-staging.houseofmoran.io` at that
    * should be built using a `Dockerfile` called `Dockerfile.skeet-feed` and follow current best practices for building and deploying Rust apps via Docker
    * this should be another `cot.rs` website
    * it should connect to the remote R2 store
    * see Bluesky docs:
        * https://docs.bsky.app/docs/api/app-bsky-feed-get-feed-skeleton
        * https://docs.bsky.app/docs/starter-templates/custom-feeds
    * [ ] write a helper that takes the `bobby.env` and syncs it with fly.io secrets
    * [ ] this should use a deploy approach which is similar, but not same, as https://github.com/mikemoraned/fosdem/blob/main/Justfile i.e.
        * deploy_staging
        * test_webapp (which uses `cot.rs` integ tests)
        * test_staging (which runs same `cot.rs` integ tests against staging, after `deploy_staging` completes)
    * [ ] write a helper rust cli that allows a Custom Feed be registered with bluesky
        * see `scripts/publishFeedGen.ts` mentioned in [docs](https://docs.bsky.app/docs/starter-templates/custom-feeds) and https://crates.io/crates/skyfeed for inspiration


* [ ] `skeet-refine` changes:
    * ...