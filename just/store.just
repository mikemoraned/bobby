LOCAL_OTEL_RESOURCE_ATTRS := "OTEL_RESOURCE_ATTRIBUTES=deployment.environment=local,service.version=" + GIT_HASH

generate-sse-c-key:
    op item create --vault Dev --title hom-bobby-r2-sse-c-key --category password "password=$(openssl rand -base64 32)" \
        || op item edit hom-bobby-r2-sse-c-key --vault Dev "password=$(openssl rand -base64 32)"

generate-session-secret:
    op item create --vault Dev --title bobby-session-secret --category password "password=$(openssl rand -hex 32)" \
        || op item edit bobby-session-secret --vault Dev "password=$(openssl rand -hex 32)"

validate-storage:
    cargo run --quiet --release --bin validate-storage -- --store-path {{ STORE }}

validate-storage-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin validate-storage -- --store-path {{ R2_STORE }}

prune:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ STORE }}

prune-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-prune {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin pruner -- --config-path config/prune.toml --store-path {{ R2_STORE }}

export-image image_id:
    cargo run --quiet --release --bin export-image -- --store-path {{ STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

export-image-r2 image_id:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin export-image -- --store-path {{ R2_STORE }} --image-id {{ image_id }} --output examples/{{ image_id }}.png

image-metadata-dump image_id:
    cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ STORE }} --image-id {{ image_id }}

image-metadata-dump-r2 image_id:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin image-metadata-dump -- --store-path {{ R2_STORE }} --image-id {{ image_id }}

at-metadata-dump at_uri:
    cargo run --quiet --release --bin at-metadata-dump -- --at-uri {{ at_uri }}

count-versions:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin count-versions -- --store-path {{ R2_STORE }}

model-versions:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin model-versions -- --store-path {{ R2_STORE }}

abort-multipart-uploads:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }}

abort-multipart-uploads-confirm:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin abort-multipart-uploads -- --store-path {{ R2_STORE }} --abort

summarise:
    cargo run --quiet --release --bin summarise -- --store-path {{ STORE }}

summarise-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin summarise -- --store-path {{ R2_STORE }}

optimise:
    cargo run --quiet --release --bin optimise -- --store-path {{ STORE }}

optimise-check:
    cargo run --quiet --release --bin optimise -- --store-path {{ STORE }} --check-only

optimise-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin optimise -- --store-path {{ R2_STORE }}

optimise-r2-check:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin optimise -- --store-path {{ R2_STORE }} --check-only

add-to-blocklist at_uri reason="manual":
    cargo run --quiet --release --bin add-to-blocklist -- "{{ at_uri }}" --reason "{{ reason }}"

capture-appraisals:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin capture-appraisals -- --store-path {{ R2_STORE }}

update-prices:
    cargo run --quiet --release --bin update-prices -- --models gpt-4o,gpt-4o-mini

train eval_output:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin train -- --store-path {{ R2_STORE }} --eval-output {{ eval_output }}

refine-eval output:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin refine-eval -- --store-path {{ R2_STORE }} --output {{ output }}

live-refine:
    op run --env-file bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ STORE }}

live-refine-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

live-refine-r2-debug:
    RUST_LOG=skeet_store=debug {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin live-refine -- --store-path {{ R2_STORE }}

classify-examples:
    cargo run --quiet --release --bin classify-examples

eval:
    cargo run --quiet --release --bin eval -- --config-path config/prune.toml --store-path {{ STORE }}

eval-r2:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin eval -- --config-path config/prune.toml --store-path {{ R2_STORE }}

eval-r2-text:
    {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --quiet --release --bin eval -- --config-path config/prune.toml --store-path {{ R2_STORE }} --categories Face,Metadata,Text

otel-test:
    OTEL_SERVICE_NAME=otel-test op run --env-file bobby-grafana-otel.env -- cargo run --quiet --release --bin otel-test

trace-summary service="skeet-live-refine" span="list_unscored_image_ids_for_version" sample="10" lookback="60":
    op run --env-file bobby-grafana-otel.env -- cargo run --quiet --release --bin trace-summary -- \
        --service {{ service }} --span {{ span }} --sample {{ sample }} --lookback-minutes {{ lookback }}

# Capture Tempo API responses as test fixtures (run once, commit the results)
capture-search-fixture service="skeet-live-refine":
    op run --env-file bobby-grafana-otel.env -- sh -c \
        'curl -s -u "$TEMPO_USER:$TEMPO_TOKEN" \
        "${TEMPO_URL}/api/search?q=%7Bresource.service.name%3D%22{{ service }}%22%7D&limit=5" \
        | jq . > skeet-store/tests/fixtures/tempo_search_response.json'

capture-trace-fixture trace_id:
    op run --env-file bobby-grafana-otel.env -- sh -c \
        'curl -s -u "$TEMPO_USER:$TEMPO_TOKEN" \
        "${TEMPO_URL}/api/traces/{{ trace_id }}" \
        | jq . > skeet-store/tests/fixtures/tempo_trace_response.json'
