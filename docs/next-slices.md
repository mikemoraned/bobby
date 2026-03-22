# Next Slices

## Slice 9: Envelope vs Deep filtering/selection

What we have effectively been doing so far is doing a bunch of quick checks to exclude 'obviously' non-matching skeets. So, biasing towards checks which allow a small %-age of positives through which may be wrong and exclude a large number of negatives. Now that we have a small (sub 1%) amount coming through, we can apply more elaborate and expensive checks on the 1%.

We will use OpenAI here, accessed via Rust API's, as our content generator. We will pass in OpenAI API keys from 1Password Dev access.

Tasks
* [ ] introduce an instruction generation step where we generate two kinds of commands (this happens manually and then is captured as config)
    * a positive command prompt which looks at all the examples in `expected.toml` which we expect to accept + the description of purpose from CLAUDE.md and generates a command which matches all positive examples
    * a negative instruction which attempts to capture, by looking at negative example images, what aspects we want to avoid
* [ ] in `finder`, we refactor it and it's modules into:
    * `envelope_classify` which is the current `classify` now categorised as an envelope step
    * `deep_classify` which is where deep classification will happen but for now it can be a no-op i.e. everything passes
* [ ] update `deep_classify` to:
    * run the image first past the command that will instruct the LLM to filter out things we don't want
    * then, if that passes, run past the positive command
    * return an Enum which summarises outcome; for now, we'll just log this out
* [ ] update `finder` to always save anything that passes the envelope stage, but add an extra column for the verdict of the `deep_classify` stage
* [ ] update `feed` to show this extra column

## Slice 10: Running `finder` remotely on Hetzner

Currently everything runs locally. We want to run `finder` on a remote single-node k3s cluster on Hetzner Cloud ARM, so it can run continuously without tying up a local machine, anmd so it is closer, in network latency/bandwidth, to what it depends on. Persistent state (e.g. the skeet store) is kept external to the cluster so we can destroy and recreate it freely.

We use `hetzner-k3s` for cluster provisioning, the 1Password Kubernetes Operator for secret injection (replacing local `op run --env-file`), and GitHub Container Registry for images.

### Tasks

- [ ] Create a multi-platform Dockerfile for `finder` that builds the Rust binary targeting `linux/arm64` (and local mac arm for local testing); document in `docs/`
- [ ] Set up GitHub Container Registry publishing (manual `docker push` is fine initially; CI can come later)
- [ ] Create `hetzner-k3s` cluster config (`infra/bobby-cluster.yaml`) for a single CAX21 master node in `fsn1` with `schedule_workloads_on_masters: true` and no worker pools; document in `docs/`
- [ ] Install 1Password Connect + Operator via Helm on the cluster; create `OnePasswordItem` resources that map the existing `bobby.env` 1Password items to k8s Secrets
- [ ] Create k8s deployment manifest for `finder` that pulls the ARM image from GHCR and injects secrets as env vars via `envFrom`
- [ ] Verify `finder` runs on the cluster, connects to Bluesky firehose, classifies images, and writes to the store
- [ ] Add `just` recipes for common remote operations (e.g. `just deploy`, `just logs`, `just cluster-create`, `just cluster-delete`)
- [ ] Document the full setup and teardown process in `docs/remote-setup.md`
