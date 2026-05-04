cloudflare-sync:
    op run --env-file cloudflare-exporter.env -- cargo run --quiet --release --bin sync

cloudflare-sync-window from to:
    op run --env-file cloudflare-exporter.env -- cargo run --quiet --release --bin sync -- --from {{ from }} --to {{ to }}
