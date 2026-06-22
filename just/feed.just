STAGING_HOSTNAME := "bobby-staging.houseofmoran.io"
PRODUCTION_HOSTNAME := "bobby.houseofmoran.io"
PUBLISHER_DID := "did:plc:cjvdzmk4iapi5p5orrasehxp"


feed:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-feed {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-publish-local.env -- cargo run --release --bin skeet-feed -- --hostname {{ STAGING_HOSTNAME }} --publisher-did {{ PUBLISHER_DID }}

deploy_feed_staging: deploy_feed_staging_secrets deploy_feed_staging_app end_to_end_test_feed_staging

deploy_feed_staging_secrets:
    op inject -i deploy/bobby-feed-staging.env | grep "^BOBBY_\|^OTEL_" | fly secrets import --config deploy/fly.staging.toml

deploy_feed_staging_app: push-skeet-feed
    fly deploy --config deploy/fly.staging.toml --image {{ GHCR_REPO }}/skeet-feed:{{ GIT_HASH }} --env "OTEL_RESOURCE_ATTRIBUTES=deployment.environment=fly,service.version={{ GIT_HASH }}"

end_to_end_test_feed_staging:
    TEST_BASE_URL=https://bobby-staging.fly.dev cargo nextest run --release -p skeet-feed --features integ --test feed_integration

deploy_feed_production: deploy_feed_production_secrets deploy_feed_production_app end_to_end_test_feed_production

deploy_feed_production_secrets:
    op inject -i deploy/bobby-feed.env | grep "^BOBBY_\|^OTEL_" | fly secrets import --config deploy/fly.production.toml

deploy_feed_production_app: push-skeet-feed
    fly deploy --config deploy/fly.production.toml --image {{ GHCR_REPO }}/skeet-feed:{{ GIT_HASH }} --env "OTEL_RESOURCE_ATTRIBUTES=deployment.environment=fly,service.version={{ GIT_HASH }}"

end_to_end_test_feed_production:
    TEST_BASE_URL=https://bobby-feed.fly.dev cargo nextest run --release -p skeet-feed --features integ --test feed_integration

register-feed:
    op run --env-file deploy/register.env -- cargo run --release --bin register-feed -- --hostname {{ STAGING_HOSTNAME }}

register-feed-production:
    op run --env-file deploy/register.env -- cargo run --release --bin register-feed -- --hostname {{ PRODUCTION_HOSTNAME }} --feed-name bobby --display-name Bobby
