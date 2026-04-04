GHCR_REPO := "ghcr.io/mikemoraned/bobby"

ghcr-login:
    op read "op://Dev/bobby-ghcr-pat-1/password" | docker login ghcr.io -u mikemoraned --password-stdin

build-pruner:
    docker buildx build --platform linux/arm64 -f Dockerfile.pruner -t {{ GHCR_REPO }}/pruner:latest .

push-pruner: build-pruner
    docker push {{ GHCR_REPO }}/pruner:latest

build-live-refine:
    docker buildx build --platform linux/arm64 -f Dockerfile.live-refine -t {{ GHCR_REPO }}/live-refine:latest .

push-live-refine: build-live-refine
    docker push {{ GHCR_REPO }}/live-refine:latest
