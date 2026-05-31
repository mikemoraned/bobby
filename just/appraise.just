APPRAISE_HOSTNAME := "bobby-appraisals-staging.houseofmoran.io"

appraise:
    RUST_BACKTRACE=1 cargo run --release --bin skeet-appraise -- --store-path {{ STORE }} --local-admin

appraise-r2:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-appraise {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file bobby-local.env -- cargo run --release --bin skeet-appraise -- --store-path {{ R2_STORE }} --local-admin

deploy_appraise: deploy_appraise_secrets deploy_appraise_app end_to_end_test_appraise

deploy_appraise_secrets:
    op inject -i bobby-appraisals-staging.env | grep "^BOBBY_\|^OTEL_" | fly secrets import --config fly.appraise-staging.toml

deploy_appraise_app: push-skeet-appraise
    fly deploy --config fly.appraise-staging.toml --image {{ GHCR_REPO }}/skeet-appraise:{{ GIT_HASH }} --env "OTEL_RESOURCE_ATTRIBUTES=deployment.environment=fly,service.version={{ GIT_HASH }}"

end_to_end_test_appraise:
    curl -fsS -o /dev/null -w "%{http_code}\n" https://{{ APPRAISE_HOSTNAME }}/
