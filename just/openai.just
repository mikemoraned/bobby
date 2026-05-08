openai-sync-costs:
    op run --env-file openai-exporter.env -- cargo run --quiet --release --bin sync_costs

integ_test_openai:
    op run --env-file openai-exporter.env -- cargo nextest run --release -p openai-exporter --run-ignored only fetch_costs_real_api
