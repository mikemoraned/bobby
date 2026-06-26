# Remote Setup (Hetzner k3s)

Bobby's pruner, live-refine, skeet-publish, and exporter/optimise cronjobs run on a single-node k3s cluster on Hetzner Cloud (CX33, Falkenstein). The cluster is disposable; persistent state lives in R2. These stable components run in a dedicated `production` namespace — see [versioning.md](versioning.md) for the production/staging split they're the foundation of. (The feed itself runs on Fly, not k8s.)

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
just build-pruner   # builds linux/amd64 image
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
just cluster-secret-install
```

This creates the `production` namespace, installs the 1Password Connect + Operator via Helm (cluster-wide, in `default`), then applies the `production`-scoped `OnePasswordItem` resources from `infra/k8s/onepassword-items.yaml` plus the `ghcr-pull-secret`. The operator (which watches all namespaces) creates the k8s Secrets in `production` from these 1Password items:

| k8s Secret name | 1Password item | Env var |
|---|---|---|
| `bobby-r2-endpoint` | `hom-bobby-r2-local-rw-endpoint` | `BOBBY_S3_ENDPOINT` |
| `bobby-r2-access-key-id` | `hom-bobby-r2-local-rw-id` | `BOBBY_S3_ACCESS_KEY_ID` |
| `bobby-r2-secret-access-key` | `hom-bobby-r2-local-rw-key` | `BOBBY_S3_SECRET_ACCESS_KEY` |
| `bobby-sse-c-key` | `n2dy5qktxi7k3ukqinoym4l6nq` | `BOBBY_SSE_C_KEY` |
| `bobby-openai-api-key` | `hom-bobby-openai-key` | `BOBBY_OPENAI_API_KEY` |
| `bobby-openai-admin-usage-key` | `bobby-openai-admin-usage-key` | `BOBBY_OPENAI_ADMIN_KEY` |
| `bobby-redis-publish-url` | `bobby-upstash-redis-publish-tcp-url` | `BOBBY_REDIS_PUBLISH_URL` |
| `bobby-grafana-oltp-endpoint` | `bobby-grafanacloud-oltp-endpoint` | `OTEL_EXPORTER_OTLP_ENDPOINT` |
| `bobby-grafana-oltp-headers` | `bobby-grafanacloud-oltp-headers` | `OTEL_EXPORTER_OTLP_HEADERS` |
| `bobby-cloudflare-analytics-token` | `bobby-cloudflare-analytics-token` | `BOBBY_CLOUDFLARE_API_TOKEN` |
| `bobby-cloudflare-account-tag` | `bobby-cloudflare-account-tag` | `BOBBY_CLOUDFLARE_ACCOUNT_TAG` |
| `bobby-grafanacloud-prom-endpoint` | `bobby-grafanacloud-prom-endpoint` | `BOBBY_PROM_ENDPOINT` |
| `bobby-grafanacloud-prom-auth` | `bobby-grafanacloud-prom-auth` | `BOBBY_PROM_AUTH` |

### Check status

```sh
just cluster-secret-status
```

## Deploy workloads

```sh
just cluster-deploy-all
```

Builds and pushes images tagged with the current git hash, then applies every `infra/k8s/*.yaml` workload into the `production` namespace. Individual components have their own `cluster-deploy-*`, `cluster-logs-*`, `cluster-restart-*`, `cluster-enable-*`/`cluster-disable-*`, and `cluster-rollback-* <image_tag>` recipes; `cluster-status` shows everything in `production`.

## Cluster config

- **Config file**: `infra/bobby-cluster.yaml`
- **Instance**: CX33 (4 vCPU x86, 8 GB RAM) in `fsn1`
- **Single master** with `schedule_workloads_on_masters: true`, no worker pools
