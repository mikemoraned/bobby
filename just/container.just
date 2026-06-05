GHCR_REPO := "ghcr.io/mikemoraned/bobby"

# md5 of every dependency-affecting file. Bumps when deps, the pinned toolchain,
# or the rustflags change. A stale hash never breaks correctness — cargo
# re-fingerprints and recompiles the delta in each service build — it only costs
# a cache miss against an out-of-date base.
DEPS_HASH := `{ echo ./Cargo.lock; find . -name Cargo.toml -not -path './target/*'; echo ./rust-toolchain.toml; echo ./.cargo/config.toml; } | LC_ALL=C sort | xargs cat | md5 -q`
DEPS_IMAGE := GHCR_REPO + "/bobby-deps:" + DEPS_HASH
DEPS_HASH_ARG := "--build-arg DEPS_HASH=" + DEPS_HASH

ghcr-login:
    op read "op://Dev/bobby-ghcr-pat-1/password" | docker login ghcr.io -u mikemoraned --password-stdin

GIT_HASH := `git rev-parse --short HEAD`
BUILD_HASH_ARG := "--build-arg BUILD_GIT_HASH=" + GIT_HASH

# Print the deps base image hash (see DEPS_HASH).
deps-hash:
    @echo {{ DEPS_HASH }}

# Cook the whole-workspace dep tree once into the multi-arch base every service FROMs.
build-deps:
    docker buildx build --platform linux/arm64,linux/amd64 -f Dockerfile.deps -t {{ DEPS_IMAGE }} .

push-deps:
    docker buildx build --platform linux/arm64,linux/amd64 -f Dockerfile.deps -t {{ DEPS_IMAGE }} --push .

# Build+push the deps base only when its tag isn't already in the registry.
push-deps-if-needed:
    @if docker manifest inspect {{ DEPS_IMAGE }} > /dev/null 2>&1; then \
        echo "deps base {{ DEPS_IMAGE }} already present, skipping build"; \
    else \
        just push-deps; \
    fi

build-pruner:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest .

push-pruner:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest -t {{ GHCR_REPO }}/pruner:{{ GIT_HASH }} --push .

build-live-refine:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest .

push-live-refine:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest -t {{ GHCR_REPO }}/live-refine:{{ GIT_HASH }} --push .

build-skeet-publish:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-publish -t {{ GHCR_REPO }}/skeet-publish:latest .

push-skeet-publish:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-publish -t {{ GHCR_REPO }}/skeet-publish:latest -t {{ GHCR_REPO }}/skeet-publish:{{ GIT_HASH }} --push .

build-optimise:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.optimise -t {{ GHCR_REPO }}/optimise:latest .

push-optimise:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.optimise -t {{ GHCR_REPO }}/optimise:latest -t {{ GHCR_REPO }}/optimise:{{ GIT_HASH }} --push .

build-skeet-feed:
    docker buildx build --platform linux/amd64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest .

push-skeet-feed:
    docker buildx build --platform linux/amd64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest -t {{ GHCR_REPO }}/skeet-feed:{{ GIT_HASH }} --push .

build-skeet-appraise:
    docker buildx build --platform linux/amd64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-appraise -t {{ GHCR_REPO }}/skeet-appraise:latest .

push-skeet-appraise:
    docker buildx build --platform linux/amd64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.skeet-appraise -t {{ GHCR_REPO }}/skeet-appraise:latest -t {{ GHCR_REPO }}/skeet-appraise:{{ GIT_HASH }} --push .

build-cloudflare-exporter:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.cloudflare-exporter -t {{ GHCR_REPO }}/cloudflare-exporter:latest .

push-cloudflare-exporter:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.cloudflare-exporter -t {{ GHCR_REPO }}/cloudflare-exporter:latest -t {{ GHCR_REPO }}/cloudflare-exporter:{{ GIT_HASH }} --push .

push-openai-exporter:
    docker buildx build --platform linux/arm64 {{ DEPS_HASH_ARG }} {{ BUILD_HASH_ARG }} -f Dockerfile.openai-exporter -t {{ GHCR_REPO }}/openai-exporter:latest -t {{ GHCR_REPO }}/openai-exporter:{{ GIT_HASH }} --push .
