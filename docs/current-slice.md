# Current Slice: Slice 10 — Running pruning/refining remotely on Hetzner

Currently everything runs locally. We want to run pruning and refining on a remote single-node k3s cluster on Hetzner Cloud ARM, so it can run continuously without tying up a local machine, and so it is closer, in network latency/bandwidth, to what it depends on. Persistent state (e.g. the skeet store) is kept external to the cluster so we can destroy and recreate it freely.

We use `hetzner-k3s` for cluster provisioning, the 1Password Kubernetes Operator for secret injection (replacing local `op run --env-file`), and GitHub Container Registry for images.

### Tasks

#### Pre-requisites (make pruner container-friendly)
- [ ] Add `--config-path` CLI arg to pruner so `config/prune.toml` path isn't hardcoded via `CARGO_MANIFEST_DIR`; consistent with the "all config via named CLI params" invariant
- [ ] Update `just prune` / `just prune-r2` recipes to pass `--config-path`

#### Dockerfile & registry
- [ ] Create a multi-platform Dockerfile for `pruner` that builds the Rust binary targeting `linux/arm64` (and local mac arm for local testing); document in `docs/`
  - `models/*.onnx` must be in the build context (ONNX→Burn conversion happens at compile time via `build.rs`)
  - The resulting `.bpk` weights file lives in the builder's `OUT_DIR` (`target/release/build/face-detection-<hash>/out/model/`); locate and copy it to the runtime image
  - Copy `config/prune.toml` into the image (used as default `--config-path`)
- [ ] Set up GitHub Container Registry publishing (manual `docker push` is fine initially; CI can come later)

#### Cluster provisioning
- [ ] Create `hetzner-k3s` cluster config (`infra/bobby-cluster.yaml`) for a single CAX21 master node in `fsn1` with `schedule_workloads_on_masters: true` and no worker pools; document in `docs/`

#### Secrets
- [ ] Install 1Password Connect + Operator via Helm on the cluster; create `OnePasswordItem` resources that map the existing `bobby.env` 1Password items to k8s Secrets
  - Secrets needed: `BOBBY_S3_ENDPOINT`, `BOBBY_S3_ACCESS_KEY_ID`, `BOBBY_S3_SECRET_ACCESS_KEY`, `BOBBY_SSE_C_KEY`, `BOBBY_OPENAI_API_KEY` (for live-refine), `OTEL_EXPORTER_OTLP_HEADERS` (for Honeycomb)

#### Deployments
- [ ] Create k8s deployment manifest for `pruner` that pulls the ARM image from GHCR, injects secrets as env vars via `envFrom`, and sets OTEL env vars (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`)
- [ ] Verify `pruner` runs on the cluster, connects to Bluesky firehose, classifies images, and writes to the store
- [ ] Create k8s deployment manifest for `live-refine` (after pruner is verified working)
- [ ] Add a PersistentVolume for `--fallback-local-store` on the pruner deployment

#### Operations & docs
- [ ] Add `just` recipes for common remote operations (e.g. `just deploy`, `just logs`, `just cluster-create`, `just cluster-delete`)
- [ ] Document the full setup and teardown process in `docs/remote-setup.md`
