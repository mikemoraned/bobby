LOCAL_OTEL_RESOURCE_ATTRS_CF := "OTEL_RESOURCE_ATTRIBUTES=deployment.environment=local,service.version=" + GIT_HASH

cloudflare-sync:
    OTEL_SERVICE_NAME=cloudflare-exporter {{ LOCAL_OTEL_RESOURCE_ATTRS_CF }} op run --env-file cloudflare-exporter.env -- cargo run --quiet --release --bin sync

cloudflare-sync-window from to:
    OTEL_SERVICE_NAME=cloudflare-exporter {{ LOCAL_OTEL_RESOURCE_ATTRS_CF }} op run --env-file cloudflare-exporter.env -- cargo run --quiet --release --bin sync -- --from {{ from }} --to {{ to }}
