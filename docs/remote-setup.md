# Remote Setup (Hetzner k3s)

Bobby's pruner and live-refine run on a single-node k3s cluster on Hetzner Cloud ARM (CAX21, Falkenstein). The cluster is disposable; persistent state lives in R2.

## Prerequisites

1. **CLI tools**: `just cluster-prerequisites` (installs needed tools e.g. hetzner-k3s and helm via brew)
2. **Hetzner Cloud API token**: stored in 1Password at `Dev/bobby-hetzner-api-token`
3. **SSH key pair**: stored in 1Password at `Dev/bobby-hetzner-ssh`

Both the API token and SSH keys are pulled from 1Password automatically by the just recipes.

## Cluster lifecycle

### Create

```sh
just cluster-create
```

This exports SSH keys from 1Password to `infra/ssh/`, reads the API token, then runs `hetzner-k3s create`. Kubeconfig is written to `infra/kubeconfig`.

### Delete

```sh
just cluster-delete
```

### Access

```sh
export KUBECONFIG=./infra/kubeconfig
kubectl get nodes
```

## Container images

Images are published to GitHub Container Registry (`ghcr.io/mikemoraned/bobby/`).

### Build and push pruner

```sh
just build-pruner   # builds linux/arm64 image
just push-pruner    # builds and pushes to GHCR
```

Authenticate to GHCR first (one-time): `just ghcr-login` (reads classic PAT from 1Password `Dev/bobby-ghcr-pat-1`)

### How the pruner image works

- `Dockerfile.pruner` is a multi-stage build: Rust builder + Debian slim runtime
- `models/*.onnx` are included in the build context (excluded from the default `.dockerignore` but included via `Dockerfile.pruner.dockerignore`)
- The `face-detection` crate's `build.rs` converts ONNX to `.bpk` weights and bakes the path into the binary; the Dockerfile locates and copies this file to the runtime image at the same path
- `config/prune.toml` is copied to `/etc/bobby/prune.toml`

## Cluster config

- **Config file**: `infra/bobby-cluster.yaml`
- **Instance**: CAX21 (4 vCPU ARM, 8 GB RAM) in `fsn1`
- **Single master** with `schedule_workloads_on_masters: true`, no worker pools
