# Current Slice: Slice 10 — Running pruning/refining remotely on Hetzner

Currently everything runs locally. We want to run pruning and refining on a remote single-node k3s cluster on Hetzner Cloud ARM, so it can run continuously without tying up a local machine, and so it is closer, in network latency/bandwidth, to what it depends on. Persistent state (e.g. the skeet store) is kept external to the cluster so we can destroy and recreate it freely.

We use `hetzner-k3s` for cluster provisioning, the 1Password Kubernetes Operator for secret injection (replacing local `op run --env-file`), and GitHub Container Registry for images.

### Tasks

#### Pre-requisites (make pruner container-friendly)
- [x] Add `--config-path` CLI arg to pruner so `config/prune.toml` path isn't hardcoded via `CARGO_MANIFEST_DIR`; consistent with the "all config via named CLI params" invariant
- [x] Update `just prune` / `just prune-r2` recipes to pass `--config-path`

#### Cluster provisioning
- [x] Create `hetzner-k3s` cluster config (`infra/bobby-cluster.yaml`) for a single CAX21 master node in `fsn1` with `schedule_workloads_on_masters: true` and no worker pools; document in `docs/`
- [x] **Manual**: Install cluster prerequisites (`just cluster-prerequisites`)
- [x] **Manual**: Create Hetzner Cloud API token (Read & Write) at console.hetzner.cloud → Security → API Tokens; store in 1Password at `Dev/bobby-hetzner-api-token`
- [x] **Manual**: Ensure SSH key pair is in 1Password at `Dev/bobby-hetzner-ssh` (just recipes export it automatically)
- [x] **Manual**: Create cluster: `just cluster-create` (pulls API token and SSH keys from 1Password; takes several minutes after instance is running while k3s is installed and configured)


#### Dockerfile & registry
- [x] Create a multi-platform Dockerfile for `pruner` (`Dockerfile.pruner`) targeting `linux/arm64`
  - `models/*.onnx` included in build context via `Dockerfile.pruner.dockerignore`
  - `.bpk` weights file located after build and copied to the path baked into the binary
  - `config/prune.toml` copied to `/etc/bobby/prune.toml` as default `--config-path`
- [x] Set up GitHub Container Registry publishing: `just build-pruner` and `just push-pruner`
- [x] **Manual**: Authenticate to GHCR: `just ghcr-login` (classic PAT stored in 1Password `Dev/bobby-ghcr-pat-1`)
- [x] **Manual**: Verify build works: `just build-pruner`

#### Secrets
- [x] Create 1Password Connect server and access token (stored in `Dev/bobby-connect-credentials` and `Dev/bobby-connect-token`)
- [x] Create `OnePasswordItem` manifests (`infra/k8s/onepassword-items.yaml`) for all bobby secrets
- [x] Add `just cluster-secrets-install` and `just cluster-secrets-status` recipes
- [x] **Manual**: Install secrets on cluster: `just cluster-secrets-install`
- [x] **Manual**: Verify secrets synced: `just cluster-secrets-status`

#### Deployments
- [ ] Create k8s deployment manifest for `pruner` that pulls the ARM image from GHCR, injects secrets as env vars via `envFrom`, and sets OTEL env vars (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`)
- [ ] Verify `pruner` runs on the cluster, connects to Bluesky firehose, classifies images, and writes to the store
- [ ] Create k8s deployment manifest for `live-refine` (after pruner is verified working)
- [ ] Add a PersistentVolume for `--fallback-local-store` on the pruner deployment

#### Operations & docs
- [ ] Add `just` recipes for common remote operations (e.g. `just deploy`, `just logs`); `just cluster-create` and `just cluster-delete` already added
- [x] Document the full setup and teardown process in `docs/remote-setup.md`

#### Refactors
- [ ] the `Justfile` is getting pretty big. Can we decompose it into smaller files (focussed on logical clusters of actions)?
