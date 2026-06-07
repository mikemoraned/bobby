appraise-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-appraise {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --release --bin skeet-appraise -- --store-path {{ R2_STORE }} --local-admin --bind 127.0.0.1:8081

deploy_appraise_staging: deploy_appraise_staging_secrets deploy_appraise_staging_app end_to_end_test_appraise_staging

deploy_appraise_staging_secrets:
    op inject -i bobby-appraisals-staging.env | grep "^BOBBY_\|^OTEL_" | fly secrets import --config fly.appraise-staging.toml

deploy_appraise_staging_app: push-skeet-appraise
    fly deploy --config fly.appraise-staging.toml --image {{ GHCR_REPO }}/skeet-appraise:{{ GIT_HASH }} --env "OTEL_RESOURCE_ATTRIBUTES=deployment.environment=fly,service.version={{ GIT_HASH }}"

end_to_end_test_appraise_staging:
    TEST_BASE_URL=https://bobby-appraisals-staging.fly.dev cargo nextest run --release -p skeet-appraise --features integ --test appraise_integration
