STAGING_HOSTNAME := "bobby-staging.houseofmoran.io"
PUBLISHER_DID := "did:plc:cjvdzmk4iapi5p5orrasehxp"

feed:
    RUST_BACKTRACE=1 cargo run --quiet --release --bin skeet-feed -- --store-path {{ STORE }} --hostname localhost:8080 --publisher-did did:web:localhost:8080

feed-r2:
    RUST_BACKTRACE=1 OTEL_EXPORTER_OTLP_ENDPOINT={{ OTEL_ENDPOINT }} OTEL_SERVICE_NAME=skeet-feed op run --env-file bobby.env -- cargo run --quiet --release --bin skeet-feed -- --store-path {{ R2_STORE }} --hostname {{ STAGING_HOSTNAME }} --publisher-did {{ PUBLISHER_DID }}

test_feed:
    cargo test --quiet --release -p skeet-feed --features test

deploy_staging: deploy_staging_secrets deploy_staging_app test_staging

deploy_staging_secrets:
    op inject -i bobby.env | grep "^BOBBY_" | fly secrets import --config fly.staging.toml

deploy_staging_app: push-skeet-feed
    fly deploy --config fly.staging.toml

test_staging:
    TEST_BASE_URL=https://bobby-staging.fly.dev cargo test --quiet --release -p skeet-feed --features test

register-feed:
    op run --env-file register.env -- cargo run --quiet --release --bin register-feed -- --hostname {{ STAGING_HOSTNAME }}
