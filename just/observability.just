# Trace tooling: summarise spans from Grafana Tempo and capture API responses as
# test fixtures for the `observability` crate.

trace-summary service="skeet-live-refine" span="list_unscored_image_ids_for_version" sample="10" lookback="60":
    op run --env-file deploy/bobby-grafana-otel.env -- cargo run --quiet --release --bin trace-summary -- \
        --service {{ service }} --span {{ span }} --sample {{ sample }} --lookback-minutes {{ lookback }}

# Capture Tempo API responses as test fixtures (run once, commit the results)
capture-search-fixture service="skeet-live-refine":
    op run --env-file deploy/bobby-grafana-otel.env -- sh -c \
        'curl -s -u "$TEMPO_USER:$TEMPO_TOKEN" \
        "${TEMPO_URL}/api/search?q=%7Bresource.service.name%3D%22{{ service }}%22%7D&limit=5" \
        | jq . > crates/skeet-store/tests/fixtures/tempo_search_response.json'

capture-trace-fixture trace_id:
    op run --env-file deploy/bobby-grafana-otel.env -- sh -c \
        'curl -s -u "$TEMPO_USER:$TEMPO_TOKEN" \
        "${TEMPO_URL}/api/traces/{{ trace_id }}" \
        | jq . > crates/skeet-store/tests/fixtures/tempo_trace_response.json'
