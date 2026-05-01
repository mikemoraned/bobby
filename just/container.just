GHCR_REPO := "ghcr.io/mikemoraned/bobby"

ghcr-login:
    op read "op://Dev/bobby-ghcr-pat-1/password" | docker login ghcr.io -u mikemoraned --password-stdin

GIT_HASH := `git rev-parse --short HEAD`
BUILD_HASH_ARG := "--build-arg BUILD_GIT_HASH=" + GIT_HASH

build-pruner:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest .

push-pruner:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest -t {{ GHCR_REPO }}/pruner:{{ GIT_HASH }} --push .

build-live-refine:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest .

push-live-refine:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest -t {{ GHCR_REPO }}/live-refine:{{ GIT_HASH }} --push .

build-compact:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.compact -t {{ GHCR_REPO }}/compact:latest .

push-compact:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.compact -t {{ GHCR_REPO }}/compact:latest -t {{ GHCR_REPO }}/compact:{{ GIT_HASH }} --push .

build-skeet-feed:
    docker buildx build --platform linux/amd64 {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest .

push-skeet-feed:
    docker buildx build --platform linux/amd64 {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest -t {{ GHCR_REPO }}/skeet-feed:{{ GIT_HASH }} --push .

build-cloudflare-exporter:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.cloudflare-exporter -t {{ GHCR_REPO }}/cloudflare-exporter:latest .

push-cloudflare-exporter:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f Dockerfile.cloudflare-exporter -t {{ GHCR_REPO }}/cloudflare-exporter:latest -t {{ GHCR_REPO }}/cloudflare-exporter:{{ GIT_HASH }} --push .
