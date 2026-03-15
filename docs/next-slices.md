# Next Slices

## Slice 6: Tweak recognition parameters and filtering

* [ ] face position:
    * from looking at some real examples which are definite non-matches, commonly a face appearing in top-middle is an anti-indicator.
    * refine as follows:
        * [ ] add a new set of Zones, still same size as one quarter of the image:
            * TOP_CENTER
            * BOTTOM_CENTER
            * LEFT_CENTER
            * RIGHT_CENTER
        * [ ] of the expanded set of Zones, only the following should match to an Archetype:
            * TOP_LEFT, TOP_RIGHT
            * BOTTOM_LEFT, BOTTOM_RIGHT
        * the expectation is that faces previously matching to TOP_LEFT or TOP_RIGHT will now match to TOP_CENTER and be dropped
* [ ] pre-filtering still perhaps being missed
    * [ ] the example `at://did:plc:wsdcu5le5birr37kohts2aqa/app.bsky.feed.post/3mh4ogusm4c23` shows up in the JS bluesky viewer with the text "The author of the quoted post has requested their posts not be displayed on external sites." which implies we should also be finding and blocking it. It's possible this is a "re-skeet" of someone else's content. However, if so, we should also ignore.
* [ ] text showing up which we should filter on
    * `examples/0f206499-82f4-48a0-bb22-0acded0982f9.png` should be filtered as Rejection::TooMuchText
    * we may need to tweak how we use text information. For example, maybe we shouldn't filter on number of detected glyphs, but instead on what percentage of the image is text?

## Slice 7: Make minimal version available online

Target:
* `skeet-finder` still running locally on demand, but saving data to the cloud
* `skeet-feed` running as a website at `bobby.houseofmoran.io` which reads from the cloud

Tasks:
* [ ] update `skeet-finder` / `skeet-store` to save data to an S3-compatible location
    * Uses https://bunny.net (docs: https://docs.bunny.net/api-reference/storage / https://bunny.net/storage/)
    * Already created a `hom-bobby` zone
    * Desired usage:
        * For local usage, read-only and read-write keys stored in 1Password, accessed via dev integration
            * via CLI arg fetched in Justfile: `READ_WRITE_KEY := \`op read "op://Dev/hom-bobby-read-write/password"\``
            * alternatively, native Rust 1Password integration preferred if available
        * S3 bucket URL passed into CLI for `skeet-finder`; should continue to work for local dirs too
    * [ ] also update `validate-storage` to work with this

* [ ] update `skeet-feed` to run on fly.io and read from the bunny S3 location
    * Secrets managed via fly secrets (`HOM_BOBBY_READ_ONLY`)
    * Read-only access only
    * Update skeet-feed to read from S3 bucket or local dir
    * Deploy manually; integ tests validate deployment
    * `bobby-prod` and `bobby-staging` versions
    * See https://github.com/mikemoraned/fosdem/blob/main/Justfile for example

* [ ] add observability:
    * [ ] switch to tokio-tracing; keep simple
        * shared code (skeet-store, shared) should be unaware of deployment context
    * [ ] `skeet-finder`: output to local `logs/` dir with timestamped files, auto-rollover; keep indicatif UI
    * [ ] `skeet-feed`: log traces to honeycomb (existing free account)
