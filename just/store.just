# Store data-plane: validation, maintenance, inspection, moderation.
# Each op has a local (`STORE`) and/or R2 (`R2_STORE`) variant.

validate-storage:
    cargo run --quiet --release --bin validate-storage -- --store-path {{ STORE }}

validate-storage-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin validate-storage -- --store-path {{ R2_STORE }}

optimise:
    cargo run --quiet --release --bin optimise -- --store-path {{ STORE }}

optimise-check:
    cargo run --quiet --release --bin optimise -- --store-path {{ STORE }} --check-only

optimise-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin optimise -- --store-path {{ R2_STORE }}

optimise-r2-check:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin optimise -- --store-path {{ R2_STORE }} --check-only

model-versions:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin model-versions -- --store-path {{ R2_STORE }}

# One-off: reconstruct historical PruneStats from saved-image timestamps.
backfill-statistics:
    cargo run --quiet --release --bin backfill-statistics -- --store-path {{ STORE }}

backfill-statistics-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin backfill-statistics -- --store-path {{ R2_STORE }}

show-prune-statistics minutes="5":
    cargo run --quiet --release --bin show-prune-statistics -- --store-path {{ STORE }} --minutes {{ minutes }}

show-prune-statistics-r2 minutes="5":
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin show-prune-statistics -- --store-path {{ R2_STORE }} --minutes {{ minutes }}

abort-multipart-uploads:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }}

abort-multipart-uploads-confirm:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }} --abort

export-image image_id:
    cargo run --quiet --release --bin export-image -- --store-path {{ STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

export-image-r2 image_id:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin export-image -- --store-path {{ R2_STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

image-metadata-dump image_id:
    cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

image-metadata-dump-r2 image_id:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ R2_STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --quiet --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

add-to-blocklist at_uri reason="manual":
    cargo run --quiet --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"
