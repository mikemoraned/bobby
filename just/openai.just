openai-sync-costs:
    op run --env-file deploy/openai-exporter.env -- cargo run --quiet --release --bin sync_costs

end_to_end_test_openai:
    op run --env-file deploy/openai-exporter.env -- cargo nextest run --release -p openai-exporter --profile end-to-end fetch_costs_end_to_end
