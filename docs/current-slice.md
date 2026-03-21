# Current Slice: Slice 7 — Make minimal version available online

Target:
* `skeet-finder` still running locally on demand, but saving data to the cloud
* `skeet-feed` running as a website at `bobby.houseofmoran.io` which reads from the cloud

Tasks:
* [x] update  `validate-storage` to save data to an S3-compatible location, but running locally
    * Uses Cloudflare R2 (docs: https://developers.cloudflare.com/r2/)
    * R2 bucket: `hom-bobby` (endpoint: `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`)
    * Desired usage:
        * keys stored in 1Password, accessed via CLI arg fetched in Justfile e.g.
        ```
        op read "op://Dev/hom-bobby-r2-local-rw/password"
        ```
        * [x] create keys needed for running locally and saving remotely:
            * Object Read/Write Token, 
                * base name: `hom-bobby-r2-local-rw`
                * keys:
                    * `hom-bobby-r2-local-rw-token`: Token (not sure what this is for, but save it anyways)
                    * `hom-bobby-r2-local-rw-id`: Access Key ID
                    * `hom-bobby-r2-local-rw-key`: Secret Access Key
                    * `hom-bobby-r2-local-rw-endpoint`: URL
        * [ ] update Justfile for `validate-storage` to use these keys
    * [x] `SkeetStore::open` accepts a URI string + storage options; works with local paths or `s3://` URIs                
    * [x] update `SkeetStore::open` to support S3 via LanceDB storage options
    * [x] update `validate-storage` to use `StoreArgs`

* [x] add encryption of content saved to cloud, using SSE-C encryption for data stored in R2:
    * Uses S3 Server-Side Encryption with Customer-Provided Keys (SSE-C)
    * R2 supports SSE-C via S3-compatible API; data encrypted at rest with our key, R2 never stores the key
    * No code changes needed in `SkeetStore` internals; encryption is transparent at storage layer
    * All LanceDB operations (search, indexing, filtering) work normally with SSE-C
    * [x] generate Encryption key: 256-bit AES key, base64-encoded
        * [x] add a Justfile rule to install `openssl` in `prerequisites` via `brew`
        * [x] add a Justfile which does following:
            * generate, via `openssl rand -base64 32`
            * store in 1Password as `hom-bobby-r2-sse-c-key`
                * this will overwrite any previous value, but this is ok as 1Password keeps a history of I need a previous version back
    * [x] integrate to `validate-storage` cli:
        * Pass two additional storage options through `StoreArgs` (only when targeting S3):
            * `aws_server_side_encryption` = `sse-c`
            * `aws_sse_customer_key_base64` = the base64 key value
        * These are passed through LanceDB `ConnectBuilder::storage_options` → Lance `object_store` crate → R2 as SSE-C headers
        * Add `--sse-c-key` optional CLI arg to `StoreArgs`; when present, inject both storage options
    * [x] remotely store name should be called `encrypted-store`
    * [x] Update `validate-storage-r2` Justfile rule: R2 commands to pass `--sse-c-key "$(op read 'op://Dev/hom-bobby-r2-sse-c-key/password')"` 

* [x] update `skeet-finder` etc to save data to an S3-compatible location
    * [x] migrate remaining binaries (`finder`, `feed`, `export-image`, `image-metadata-dump`) to `StoreArgs`
        * pull out shared behaviour into the `skeet-store` where possible
    * [x] update Justfile with R2 commands / variables

* [x] add observability basics:
    * [x] switch to tokio-tracing
        * keep it simple as we can, but perhaps research/look for "starter" crates which provide good defaults
        * shared code (skeet-store, shared) should be unaware of deployment context
        * any errors/warnings should be logged as such
    * [x] `skeet-finder` and `skeet-feed`: 
        * should output to a local `logs/` dir with timestamped files, auto-rollover
        * keep indicatif UI's where present
    * [x] do refactoring / clarification pass on all clis:
        * split out large main files (that don't fit easily on one screen) to separate modules
        * generally reread and apply the Rust local rules where possible
    * [x] look through the different cli mains and apply INFO logging for when major steps start and end

* [x] do a general refactoring pass, applying rules
    * [x] for example, should we apply the NewType pattern to where we are doing stuff with `at` urls in multiple places to construct/extract things?
    * [x] look for any other opportunities for refactors/tidyups
        * [x] eliminate redundant face detection in `classify_image`: `classify()` already runs `detector.detect()`, but `classify_image` calls it a second time to get the face for annotation — restructure to avoid duplicate ML inference
        * [x] deduplicate excluded-labels constants: `EXCLUDED_LABELS` in `firehose.rs` and `BLOCKED_LABEL_VALUES` in `content_filter.rs` are identical lists — extract to a single constant in a `labels` module inside `shared`
        * [x] fix `ImageId::as_str()` returning `String` instead of `&str`: violates Rust `as_str()` conventions — call sites should use `Display`/`.to_string()` instead
        * [x] extract shared tracing file-appender setup: `finder/main.rs` and `skeet-feed/main.rs` have near-identical tracing init with daily file appender — add a file-appender variant to `shared::tracing`
        * [x] embed `StoredImageSummary` inside `StoredImage`: `StoredImage` duplicates all 7 summary fields — compose instead to reduce duplication and simplify `batches_to_stored_images`

* [x] add tracing of load/save for performance checking:
    * [x] add trace annotations to methods that are on the paths to saving a `StoredImage` and reading them, across all crates
        * I am particularly interested in paths that affect `skeet-feed` and `skeet-finder` clis and go through `skeet-store` to and from R2
    * [x] set up opentelemetry as a publish destination
        * should use standard env keys by default
            * if no env keys are provided then a warning should be logged and opentelemetry disabled. however, cli should still start
        * send traces/logs to honeycomb.io (see https://docs.honeycomb.io/send-data and related)
        * OTEL_EXPORTER_OTLP_ENDPOINT="https://api.honeycomb.io"
        * OTEL_EXPORTER_OTLP_HEADERS="x-honeycomb-team=<ingest key>"
            * ingest api key is stored in `hom-bobby-hcoltp-local-ingest` in 1Password
        * [x] update Justfile `feed`/`feed-r2` rules to pass above env vars + OTEL_SERVICE_NAME="skeet-feed"
        * [x] update Justfile `find`/`find-r2` rules to pass above env vars + OTEL_SERVICE_NAME="skeet-finder"

* [x] add tokio-console for local trace inspection
    * install: `cargo install tokio-console`
    * add `console-subscriber = "0.5"` to relevant binary crates (must match `tokio-console` 0.1.14; both use `console-api` 0.9)
    * call `console_subscriber::init()` in main (can coexist with existing tracing layers)
    * enable tokio's unstable tracing: compile with `RUSTFLAGS="--cfg tokio_unstable"`
    * run `tokio-console` in a separate terminal to connect to the app (default `127.0.0.1:6669`)
    * gives live TUI view of spawned tasks, poll times, waker counts, resources
    * local only, no OTLP; uses its own gRPC protocol

* [ ] update `skeet-feed` to run on fly.io and read from R2
    * Secrets managed via fly secrets (R2 read-only API token)
    * Read-only access only
    * Update skeet-feed to read from S3 bucket or local dir
    * Deploy manually; integ tests validate deployment
    * `bobby-prod` and `bobby-staging` versions
    * See https://github.com/mikemoraned/fosdem/blob/main/Justfile for example

