GHCR_REPO := "ghcr.io/mikemoraned/bobby"

ghcr-login:
    op read "op://Dev/bobby-ghcr-pat-1/password" | docker login ghcr.io -u mikemoraned --password-stdin

GIT_HASH := `git rev-parse --short HEAD`
BUILD_HASH_ARG := "--build-arg BUILD_GIT_HASH=" + GIT_HASH

# Services share one builder stage per platform (deploy/Dockerfile.cluster for arm64,
# deploy/Dockerfile.fly for amd64) and are selected with `--target runner-<name>`, so the
# dep tree compiles once and later images reuse the cached builder. See
# .claude/rules/docker.md "Shared builder + --target".

build-pruner:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-pruner -t {{ GHCR_REPO }}/pruner:latest .

push-pruner:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-pruner -t {{ GHCR_REPO }}/pruner:latest -t {{ GHCR_REPO }}/pruner:{{ GIT_HASH }} --push .

build-live-refine:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-live-refine -t {{ GHCR_REPO }}/live-refine:latest .

push-live-refine:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-live-refine -t {{ GHCR_REPO }}/live-refine:latest -t {{ GHCR_REPO }}/live-refine:{{ GIT_HASH }} --push .

build-skeet-publish:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-skeet-publish -t {{ GHCR_REPO }}/skeet-publish:latest .

push-skeet-publish:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-skeet-publish -t {{ GHCR_REPO }}/skeet-publish:latest -t {{ GHCR_REPO }}/skeet-publish:{{ GIT_HASH }} --push .

build-optimise:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-optimise -t {{ GHCR_REPO }}/optimise:latest .

push-optimise:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-optimise -t {{ GHCR_REPO }}/optimise:latest -t {{ GHCR_REPO }}/optimise:{{ GIT_HASH }} --push .

build-cloudflare-exporter:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-cloudflare-exporter -t {{ GHCR_REPO }}/cloudflare-exporter:latest .

push-cloudflare-exporter:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-cloudflare-exporter -t {{ GHCR_REPO }}/cloudflare-exporter:latest -t {{ GHCR_REPO }}/cloudflare-exporter:{{ GIT_HASH }} --push .

push-openai-exporter:
    docker buildx build --platform linux/arm64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.cluster --target runner-openai-exporter -t {{ GHCR_REPO }}/openai-exporter:latest -t {{ GHCR_REPO }}/openai-exporter:{{ GIT_HASH }} --push .

build-skeet-feed:
    docker buildx build --platform linux/amd64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.fly --target runner-skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest .

push-skeet-feed:
    docker buildx build --platform linux/amd64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.fly --target runner-skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest -t {{ GHCR_REPO }}/skeet-feed:{{ GIT_HASH }} --push .

build-skeet-appraise:
    docker buildx build --platform linux/amd64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.fly --target runner-skeet-appraise -t {{ GHCR_REPO }}/skeet-appraise:latest .

push-skeet-appraise:
    docker buildx build --platform linux/amd64 {{ BUILD_HASH_ARG }} -f deploy/Dockerfile.fly --target runner-skeet-appraise -t {{ GHCR_REPO }}/skeet-appraise:latest -t {{ GHCR_REPO }}/skeet-appraise:{{ GIT_HASH }} --push .
