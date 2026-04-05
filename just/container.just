GHCR_REPO := "ghcr.io/mikemoraned/bobby"
BOBBY_CHEF := "bobby-chef:latest"
BOBBY_RUNNER := GHCR_REPO / "bobby-runner:latest"

_ensure-multiarch-builder:
    docker buildx inspect bobby-multiarch >/dev/null 2>&1 || docker buildx create --name bobby-multiarch
    docker buildx use bobby-multiarch

ghcr-login:
    op read "op://Dev/bobby-ghcr-pat-1/password" | docker login ghcr.io -u mikemoraned --password-stdin

build-base-chef: _ensure-multiarch-builder
    docker buildx build --platform linux/arm64,linux/amd64 -f Dockerfile.bobby-chef -t {{ BOBBY_CHEF }} .

build-base-runner: _ensure-multiarch-builder
    docker buildx build --platform linux/arm64,linux/amd64 -f Dockerfile.bobby-runner -t {{ BOBBY_RUNNER }} .

build-base: build-base-chef build-base-runner

push-base: _ensure-multiarch-builder
    docker buildx build --platform linux/arm64,linux/amd64 -f Dockerfile.bobby-runner -t {{ BOBBY_RUNNER }} --push .

build-pruner:
    docker buildx build --platform linux/arm64 --build-arg BOBBY_CHEF={{ BOBBY_CHEF }} --build-arg BOBBY_RUNNER={{ BOBBY_RUNNER }} -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest .

push-pruner: build-pruner
    docker push {{ GHCR_REPO }}/pruner:latest

build-live-refine:
    docker buildx build --platform linux/arm64 --build-arg BOBBY_CHEF={{ BOBBY_CHEF }} --build-arg BOBBY_RUNNER={{ BOBBY_RUNNER }} -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest .

push-live-refine: build-live-refine
    docker push {{ GHCR_REPO }}/live-refine:latest

build-skeet-feed:
    docker buildx build --platform linux/amd64 --build-arg BOBBY_CHEF={{ BOBBY_CHEF }} --build-arg BOBBY_RUNNER={{ BOBBY_RUNNER }} -f Dockerfile.skeet-feed -t {{ GHCR_REPO }}/skeet-feed:latest .

push-skeet-feed: build-skeet-feed
    docker push {{ GHCR_REPO }}/skeet-feed:latest
