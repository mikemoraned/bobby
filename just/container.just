GHCR_REPO := "ghcr.io/mikemoraned/bobby"

ghcr-login:
    op read "op://Dev/bobby-ghcr-pat-1/password" | docker login ghcr.io -u mikemoraned --password-stdin

build-pruner:
    docker buildx build --platform linux/arm64 -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest .

push-pruner:
    docker buildx build --platform linux/arm64 -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest --push .

build-live-refine:
    docker buildx build --platform linux/arm64 -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest .

push-live-refine:
    docker buildx build --platform linux/arm64 -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest --push .

build-bench-firehose:
    docker buildx build --platform linux/arm64 -f Dockerfile.bench-firehose -t {{ GHCR_REPO }}/bench-firehose:latest .

push-bench-firehose:
    docker buildx build --platform linux/arm64 -f Dockerfile.bench-firehose -t {{ GHCR_REPO }}/bench-firehose:latest --push .

build-compact:
    docker buildx build --platform linux/arm64 -f Dockerfile.compact -t {{ GHCR_REPO }}/compact:latest .

push-compact:
    docker buildx build --platform linux/arm64 -f Dockerfile.compact -t {{ GHCR_REPO }}/compact:latest --push .

build-skeet-feed:
    docker buildx build --platform linux/amd64 -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest .

push-skeet-feed:
    docker buildx build --platform linux/amd64 -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest --push .
