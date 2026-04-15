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

- `Dockerfile.pruner` is self-contained with inline cargo-chef stages (planner/builder/runner), based on `lukemathwalker/cargo-chef:latest-rust-1-bookworm` and `debian:bookworm-slim`
- `models/*.onnx` are included in the build context (not excluded by `.dockerignore`)
- The `face-detection` crate's `build.rs` converts ONNX to `.bpk` weights and bakes the path into the binary; the Dockerfile locates and copies this file to the runtime image at the same path
- `config/prune.toml` is copied to `/etc/bobby/prune.toml`

## Secrets

Secrets are injected into the cluster via the [1Password Kubernetes Operator](https://github.com/1Password/onepassword-operator). The operator runs a Connect server inside the cluster that syncs 1Password items to k8s Secrets.

### Prerequisites (one-time)

A 1Password Connect server and access token must exist:
- **Connect credentials**: stored in 1Password at `Dev/bobby-connect-credentials` (notesPlain field, JSON)
- **Connect token**: stored in 1Password at `Dev/bobby-connect-token`

To create these from scratch:
```sh
op connect server create bobby-connect --vaults Dev
# Save the 1password-credentials.json contents to Dev/bobby-connect-credentials
op connect token create "bobby-k8s-operator" --server bobby-connect --vault Dev
# Save the token to Dev/bobby-connect-token
```

### Install operator and apply secrets

```sh
just cluster-secrets-install
```

This installs the 1Password Connect + Operator via Helm, then applies `OnePasswordItem` resources from `infra/k8s/onepassword-items.yaml`. The operator creates k8s Secrets from the following 1Password items:

| k8s Secret name | 1Password item | Env var |
|---|---|---|
| `bobby-r2-endpoint` | `hom-bobby-r2-local-rw-endpoint` | `BOBBY_S3_ENDPOINT` |
| `bobby-r2-access-key-id` | `hom-bobby-r2-local-rw-id` | `BOBBY_S3_ACCESS_KEY_ID` |
| `bobby-r2-secret-access-key` | `hom-bobby-r2-local-rw-key` | `BOBBY_S3_SECRET_ACCESS_KEY` |
| `bobby-sse-c-key` | `n2dy5qktxi7k3ukqinoym4l6nq` | `BOBBY_SSE_C_KEY` |
| `bobby-openai-api-key` | `hom-bobby-openai-key` | `BOBBY_OPENAI_API_KEY` |
| `bobby-otel-headers` | `hom-bobby-hcoltp-local-ingest` | `OTEL_EXPORTER_OTLP_HEADERS` |

### Check status

```sh
just cluster-secrets-status
```

## Cluster config

- **Config file**: `infra/bobby-cluster.yaml`
- **Instance**: CAX21 (4 vCPU ARM, 8 GB RAM) in `fsn1`
- **Single master** with `schedule_workloads_on_masters: true`, no worker pools
