# Run the firehose pruner locally (classify images, save matches).
# image_workers tunes the parallel image-classification pool (matches the binary default).

prune image_workers="2":
    RUST_BACKTRACE=1 cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ STORE }} --image-workers {{ image_workers }}

prune-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-prune {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ R2_STORE }}
