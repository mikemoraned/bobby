# Current Slice: Slice 7 — Make minimal version available online

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
