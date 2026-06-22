cloudflare-sync-operations:
    op run --env-file deploy/cloudflare-operations-exporter.env -- cargo run --quiet --release --bin sync_operations

cloudflare-sync-operations-window from to:
    op run --env-file deploy/cloudflare-operations-exporter.env -- cargo run --quiet --release --bin sync_operations -- --from {{ from }} --to {{ to }}

cloudflare-sync-storage:
    op run --env-file deploy/cloudflare-storage-exporter.env -- cargo run --quiet --release --bin sync_storage

end_to_end_test_cloudflare: end_to_end_test_cloudflare_operations end_to_end_test_cloudflare_storage

end_to_end_test_cloudflare_operations:
    op run --env-file deploy/cloudflare-operations-exporter.env -- cargo nextest run --release -p cloudflare-exporter --profile end-to-end fetch_r2_operations_end_to_end

end_to_end_test_cloudflare_storage:
    op run --env-file deploy/cloudflare-storage-exporter.env -- cargo nextest run --release -p cloudflare-exporter --profile end-to-end fetch_bucket_usage_end_to_end
