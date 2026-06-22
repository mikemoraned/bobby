# Local end-to-end testing.
#
# Runs a Redis-compatible valkey in docker that the publisher / feed / appraise
# point at instead of Upstash, while still reading the real remote R2 dataset
# (via the op-injected BOBBY_S3_* secrets). This exercises the whole
# publish -> read pipeline — including the live Bluesky existence/dimension
# probes — without touching the shared staging redis.
#
# Typical flow:
#   just valkey-up
#   just publish-local      # populate valkey from R2 (one cycle)
#   just feed-local         # serve the feed/grid, or:
#   just appraise-local     # serve the appraise UI
#   just valkey-down

LOCAL_REDIS := "redis://127.0.0.1:6379"

# Start a local valkey (Redis-compatible) container on 6379.
valkey-up:
    docker run --rm -d --name bobby-valkey -p 6379:6379 valkey/valkey:8

# Stop and remove the local valkey container.
valkey-down:
    docker stop bobby-valkey

# Tail the local valkey logs.
valkey-logs:
    docker logs -f bobby-valkey

# Open a redis-cli against the local valkey.
valkey-cli:
    docker exec -it bobby-valkey valkey-cli

# Publish one cycle from real R2 into local valkey (live Bluesky existence/dimension probes).
publish-local:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-publish {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-publish-local.env -- cargo run --release --bin skeet-publish -- --store-path {{ R2_STORE }} --redis-publish-url {{ LOCAL_REDIS }} --publish recency-48h --publish quality-48h --publish quality-7d --publish quality-4w --publish recency-4w --publish quality-1y --publish recency-1y --once

# Serve the Bluesky feed + public grid locally, reading the local valkey.
feed-local:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-feed {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-publish-local.env -- cargo run --release --bin skeet-feed -- --hostname {{ STAGING_HOSTNAME }} --publisher-did {{ PUBLISHER_DID }} --redis-publish-url {{ LOCAL_REDIS }}

# Serve the appraise UI locally (local admin) on :8081, reading local valkey + real R2.
appraise-local:
    RUST_BACKTRACE=1 OTEL_SERVICE_NAME=skeet-appraise {{ LOCAL_OTEL_RESOURCE_ATTRS }} op run --env-file deploy/bobby-local.env -- cargo run --release --bin skeet-appraise -- --store-path {{ R2_STORE }} --redis-publish-url {{ LOCAL_REDIS }} --local-admin --bind 127.0.0.1:8081
