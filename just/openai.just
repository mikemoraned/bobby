openai-sync-costs:
    op run --env-file deploy/openai-exporter.env -- cargo run --quiet --release --bin sync_costs

# Refresh the cached OpenAI model prices used for cost estimation.
update-prices:
    cargo run --quiet --release --bin update-prices -- --models gpt-4o,gpt-4o-mini,gpt-4.1-mini,gpt-4.1-nano,gpt-5,gpt-5-mini,gpt-5-nano

end_to_end_test_openai:
    op run --env-file deploy/openai-exporter.env -- cargo nextest run --release -p openai-exporter --profile end-to-end fetch_costs_end_to_end
