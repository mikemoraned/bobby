generate-sse-c-key:
    op item create --vault Dev --title hom-bobby-r2-sse-c-key --category password "password=$(openssl rand -base64 32)" \
        || op item edit hom-bobby-r2-sse-c-key --vault Dev "password=$(openssl rand -base64 32)"

generate-session-secret:
    op item create --vault Dev --title bobby-session-secret --category password "password=$(openssl rand -hex 32)" \
        || op item edit bobby-session-secret --vault Dev "password=$(openssl rand -hex 32)"

validate-storage:
    cargo run --quiet --release --bin validate-storage -- --store-path {{ STORE }}

validate-storage-r2:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin validate-storage -- --store-path {{ R2_STORE }}

prune:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ STORE }}

prune-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-prune op run --env-file bobby-local.env -- cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ R2_STORE }} --fallback-local-store {{ FALLBACK_STORE }}

inspect:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin skeet-inspect -- --store-path {{ STORE }}

inspect-fallback:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin skeet-inspect -- --store-path {{ FALLBACK_STORE }}

inspect-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-inspect op run --env-file bobby-local.env -- cargo run --quiet --release --bin skeet-inspect -- --store-path {{ R2_STORE }}

export-image image_id:
    cargo run --quiet --release --bin export-image -- --store-path {{ STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

export-image-r2 image_id:
    cargo run --quiet --release --bin export-image -- --store-path {{ R2_STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

image-metadata-dump image_id:
    cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

image-metadata-dump-r2 image_id:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ R2_STORE }} --image-id {{ image_id }}

image-metadata-dump-fallback image_id:
    cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ FALLBACK_STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --quiet --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

redrive-r2:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin redrive -- --source-store-path {{ FALLBACK_STORE }} --store-path {{ R2_STORE }} --mode upload-and-delete

redrive-local-to-r2:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin redrive -- --source-store-path {{ STORE }} --store-path {{ R2_STORE }} --mode upload --most-recent-first

abort-multipart-uploads:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }}

abort-multipart-uploads-confirm:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }} --abort

summarise:
    cargo run --quiet --release --bin summarise -- --store-path {{ STORE }}

summarise-r2:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin summarise -- --store-path {{ R2_STORE }}

compact table="all":
    cargo run --quiet --release --bin compact -- --store-path {{ STORE }} --table {{ table }}

compact-check:
    cargo run --quiet --release --bin compact -- --store-path {{ STORE }} --check-only

compact-fallback table="all":
    cargo run --quiet --release --bin compact -- --store-path {{ FALLBACK_STORE }} --table {{ table }}

compact-r2 table="all":
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin compact -- --store-path {{ R2_STORE }} --table {{ table }}

compact-r2-check:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin compact -- --store-path {{ R2_STORE }} --check-only

add-to-blocklist at_uri reason="manual":
    cargo run --quiet --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"

train:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin train

refine:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin refine -- --store-path {{ STORE }}

refine-r2:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin refine -- --store-path {{ R2_STORE }}

live-refine:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ STORE }}

live-refine-r2:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

live-refine-r2-debug:
    RUST_LOG=skeet_store=debug op run --env-file bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

classify-examples:
    cargo run --quiet --release --bin classify-examples

bench-firehose:
    cargo run --quiet --release --bin bench-firehose
