cloudflare-sync-operations:
    op run --env-file cloudflare-operations-exporter.env -- cargo run --quiet --release --bin sync_operations

cloudflare-sync-operations-window from to:
    op run --env-file cloudflare-operations-exporter.env -- cargo run --quiet --release --bin sync_operations -- --from {{ from }} --to {{ to }}

cloudflare-sync-storage:
    op run --env-file cloudflare-storage-exporter.env -- cargo run --quiet --release --bin sync_storage

integ_test_cloudflare: integ_test_cloudflare_operations integ_test_cloudflare_storage

integ_test_cloudflare_operations:
    op run --env-file cloudflare-operations-exporter.env -- cargo nextest run --release -p cloudflare-exporter --run-ignored only fetch_r2_operations_real_api

integ_test_cloudflare_storage:
    op run --env-file cloudflare-storage-exporter.env -- cargo nextest run --release -p cloudflare-exporter --run-ignored only fetch_bucket_usage_real_api
