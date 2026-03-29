# Next Slices

## Slice 9: "Bobby Dev" Custom Feed in Bluesky

### Target

* a "Bobby Dev" [Custom Feed](https://docs.bsky.app/docs/starter-templates/custom-feeds) in Bluesky which I can use for live dev testing
    * this should surface the top N scored skeets, ordered by score, where score threshold > T, and have been in last H hours
        * initially, N = 10, T = 0.5, H = 48

* [ ] refactors / cleanups
    * [ ] rename `skeet-feed` to `skeet-inspect` capturing it's role of allowing inspection of what's been found. It doesn't need to be the actually exposed feed, so rename so I can that name again for the main feed.

## Slice 10: Running pruning/refining remotely on Hetzner

Currently everything runs locally. We want to run pruning and refining on a remote single-node k3s cluster on Hetzner Cloud ARM, so it can run continuously without tying up a local machine, and so it is closer, in network latency/bandwidth, to what it depends on. Persistent state (e.g. the skeet store) is kept external to the cluster so we can destroy and recreate it freely.

We use `hetzner-k3s` for cluster provisioning, the 1Password Kubernetes Operator for secret injection (replacing local `op run --env-file`), and GitHub Container Registry for images.

### Tasks

- [ ] Create a multi-platform Dockerfile for `pruner` that builds the Rust binary targeting `linux/arm64` (and local mac arm for local testing); document in `docs/`
- [ ] Set up GitHub Container Registry publishing (manual `docker push` is fine initially; CI can come later)
- [ ] Create `hetzner-k3s` cluster config (`infra/bobby-cluster.yaml`) for a single CAX21 master node in `fsn1` with `schedule_workloads_on_masters: true` and no worker pools; document in `docs/`
- [ ] Install 1Password Connect + Operator via Helm on the cluster; create `OnePasswordItem` resources that map the existing `bobby.env` 1Password items to k8s Secrets
- [ ] Create k8s deployment manifest for `pruner` that pulls the ARM image from GHCR and injects secrets as env vars via `envFrom`
- [ ] Verify `pruner` runs on the cluster, connects to Bluesky firehose, classifies images, and writes to the store
- [ ] Add `just` recipes for common remote operations (e.g. `just deploy`, `just logs`, `just cluster-create`, `just cluster-delete`)
- [ ] Document the full setup and teardown process in `docs/remote-setup.md`
