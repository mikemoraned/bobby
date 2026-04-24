STAGING_HOSTNAME := "bobby-staging.houseofmoran.io"
PUBLISHER_DID := "did:plc:cjvdzmk4iapi5p5orrasehxp"

feed:
    RUST_BACKTRACE=1 cargo run --release --bin skeet-feed -- --store-path {{ STORE }} --hostname localhost:8080 --publisher-did did:web:localhost:8080 --local-admin

feed-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-feed {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --release --bin skeet-feed -- --store-path {{ R2_STORE }} --hostname {{ STAGING_HOSTNAME }} --publisher-did {{ PUBLISHER_DID }} --local-admin

test_feed:
    cargo test --release -p skeet-feed --features test

integ_test_feed:
    TEST_BASE_URL=http://127.0.0.1:8080 cargo nextest run --release -p skeet-feed --features integ --test feed_integration

integ_test_redis:
    cargo nextest run --release -p skeet-feed --features integ --test redis_session

deploy_staging: deploy_staging_secrets deploy_staging_app integ_test_staging

deploy_staging_secrets:
    op inject -i bobby-staging.env | grep "^BOBBY_\|^OTEL_" | fly secrets import --config fly.staging.toml

deploy_staging_app: push-skeet-feed
    fly deploy --config fly.staging.toml --image {{ GHCR_REPO }}/skeet-feed:{{ GIT_HASH }}

integ_test_staging:
    TEST_BASE_URL=https://bobby-staging.fly.dev cargo test --release -p skeet-feed --features integ --test feed_integration

register-feed:
    op run --env-file register.env -- cargo run --release --bin register-feed -- --hostname {{ STAGING_HOSTNAME }}
