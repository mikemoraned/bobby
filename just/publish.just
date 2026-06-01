# Publish feed lists from R2 to the Upstash publish server once, then log what
# landed. Reads from R2 and writes to redis — a local proof that publishing works.
publish-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-publish {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-publish-local.env -- cargo run --release --bin skeet-publish -- --store-path {{ R2_STORE }} --publish recency-48h --once
