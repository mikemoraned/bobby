# Run the firehose pruner locally (classify images, save matches).

prune:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ STORE }}

prune-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-prune {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ R2_STORE }}
