# Next Slices

## Slice: move to 1.0

### Target

There are still many things I can do here, but I'd like to get to a 1.0 version, where I have:
* bobby.houseofmoran.io: same underlying code as the staging version, but:
    * published as a feed called "Bobby" (i.e. not "Bobby Dev") in bluesky
        * a small inline blurb explaining what this is, which is shared with website (see below)
    * tracking of usage via plausible.io
    * a [social media preview image](https://support.metropublisher.com/hc/en-us/articles/31523564070420-Preview-Image-Settings-for-Social-Media) which can be shown on facebook, twitter etc.
        * this should be calculated dynamically based on same `quality-7d` content, and cached using same last-modified caching from elsewhere. 
        * We can use something like the layout algorithms used in [linzer](https://github.com/mikemoraned/geo/blob/main/apps/linzer/backend/layout/src/bin/layout.rs) e.g. `Guillotine` from `binpack2d` crate.
    * a small banner at top which shows:
        * an explanation of what this is (see blurb)
        * small qr code for `https://bobby.houseofmoran.io/` url
        * instructions on how to subscribe to the feed on bluesky (with a link to it)
        * summary data of how many images examined (should be precalculated by publisher and saved in redis)
* bobby-appraisals.houseofmoran.io: same underlying code as the staging version
* a separation between production and staging setups:
    * Ideally I'd like a setup where there is a `production` environment (perhaps represented as a namespace in k8s) which contains the stable components I don't want to break. Then, have a per-worktree staging setup where I can create a new worktree and, if I want to, have a unique set of components for that worktree.
    * However, I don't want to duplicate components for every worktree. I'd like to have something like:
        * services and jobs which can share backend data stores like R2 across envs. So, there is not a "staging" R2 store or Redis, but instead, where possible, we use versioning of tables and collections to give safe-ish separation. I say safe-ish as there is still a possibility that staging components could interfere with prod components. However, having a fully separate env for each staging setup is costly and also means any staging setup starts from scratch with data, which is likely not useful for quick iteration.
        * this versioning approach should extend into models; so we probably should have a `production` label and a label per-contender
        * this also means we should have a more explicit "promotion" process where we use model or k8s labels, or similar, to promote something developed on a branch into a main prod version. This should be supported by local cli lifecycle commands and/or third-party tools.
            * this also applies to crate versions i.e. in this slice everything should become 1.0 and then 
        * build and deployment can continue to be done locally on my laptop
* refactor, review and minimisation of code for longer-term maintainance:
    * each crate should have at least one human pass where all code is inspected, and deleted/reworked as needed
    * the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising or lingering cruft/weirdness
    * split out code into sub-dirs based on role e.g. crates are at top-level in repo, and so should go into a subdir; follow generally accepted conventions where possible
    * refactor `just` rules into more logical chunks, and do a pass to remove any that no-longer make sense

## Slice: try using embeddings for classification/scoring in refine

### Target

Refine currently routes every image through an LLM scorer — expensive, slow, and the prompt is the thing we're trying to optimise. Alternative worth measuring: embed each image once with a pre-trained vision model and learn a linear classifier on the embeddings. The embedding does the heavy lifting (visually/semantically similar → close in embedding space), so a linear SVM or logistic regression on top usually recovers a calibrated good/bad score that maps onto Low/MedLow/MedHigh/High bands. Inference drops from ~seconds and per-call cost to milliseconds and free; the prompt-optimisation problem disappears.

- **Deterministic — kills the variance problem.** A frozen embedding plus a seeded linear classifier produces a reproducible score: no temperature noise, no rewriter stochasticity, none of the 0.696–0.870 run-to-run recall spread that made the phase-4 gate hard to read. The re-run-N-times-and-take-a-confidence-interval machinery the LLM path needs just goes away.
- **Embed once, store forever.** Compute each image's embedding a single time and persist it (a new column/table in the lance store); re-scoring under a new classifier is then milliseconds and zero API calls. Contrast the LLM path, where re-scoring the ~34k stored images under a new model is exactly what CrashLoopBackOff'd live-refine (current-slice backfill incident) — here, retraining the classifier and re-scoring everything is trivial.
- **Embedding model matters more than the classifier.** [OpenCLIP](https://github.com/mlfoundations/open_clip) or [SigLIP](https://huggingface.co/docs/transformers/en/model_doc/siglip) for broad visual-semantic; [DinoV2](https://github.com/facebookresearch/dinov2) for pure-visual fine-grained. If the embedding can't see the distinction, nothing downstream recovers it.
- **Calibrated probabilities are the point, not a bonus.** A previous slice showed the cheap LLMs discriminated fine but were mis-calibrated — their scores didn't reach the 0.800-precision operating point without dumping recall. A learned head that emits calibrated 0–1 probabilities natively attacks exactly that binding constraint.
- **Classifier choices.** Logistic regression via [`linfa-logistic`](https://crates.io/crates/linfa-logistic) gives calibrated 0–1 probabilities natively (preferred); linear SVM via [`linfa-svm`](https://crates.io/crates/linfa-svm) + Platt scaling is the alternative; kNN against curated prototypes is a no-training baseline; one-class SVM / SVDD if "bad" is too diffuse to label well.
- **Runtime in Rust.** Embeddings via [`ort`](https://crates.io/crates/ort) (same dep we'd add for the skin-detection slice) or [`candle`](https://github.com/huggingface/candle) for pure-Rust. CPU-only on hetzner (no GPU) — CLIP/SigLIP base is ~tens-of-ms/image, fine at current firehose volume and folds into the fixed monthly cluster cost.
- **Tradeoff + hybrid.** Gives up controllability over *which* aspect of similarity matters; if good/bad hinges on tone/intent the embedding can't see, the LLM scorer still wins. So treat it as the primary gate with the LLM kept for borderline cases — which is also the safe cost bet: if the classifier confidently decides ~80% of images and only the ~20% near the boundary escalate to the LLM, that's an ~80% cost cut without betting everything on full replacement.

### Phase 1: decisive offline experiment (cheap to falsify)

Retire the central risk in an afternoon before building anything, reusing the slice-16 `eval` crate end-to-end:

* [ ] Embed the ~685 appraised images with 2–3 candidate models (e.g. SigLIP, OpenCLIP, DINOv2); cache embeddings to disk/store so every later step is instant
* [ ] Train `linfa-logistic` (cross-validated) on the **same frozen 143-image split** used in phases 2–4, labels from `Band::is_visible_in_feed()`
* [ ] Evaluate on the held-out test set and compare **recall-at-pinned-precision** against the deployed LLM baseline (0.870 @ P=0.800) — same gate as phase 4, so directly comparable
* [ ] Caveat: the split has only ~88 positive training examples (~16%), thin for a learned head — if logreg underperforms, try kNN/one-class before concluding the embedding can't see the distinction. This is where the label-growth bullet (refine slice) pays off most.

## Slice: improving prune and refine quality

### Target: prune

I'm still seeing some examples (e.g. examples/v2:de210c2970ed76cf79c27d8cd557214a.png) where the text-detection should ideally be excluding them. I think we can exclude these images by looking at overlap between the text bounding boxes and the 3x3 grid of zones and looking at some features:
1. what %-age of a Zone is taken up by text-boxes (unioned area)?
2. how many Zones have at least some %-age of text-box area?

We can then exclude any images that have > threshold %-age in any Zone, and > number threshold of Zones.

### Target: refine

Ways to improve refine quality and cost, distilled from previous "Slice 16 — make costs visible and reduce them" slice.

**Operating-point preference (governs how every candidate is judged).** The precision floor (0.800) is firm — false positives are the user-visible cost in the feed, so dropping below it is never an acceptable trade. Recall, by contrast, is negotiable: a candidate that holds the precision floor at meaningfully lower cost may lose *some* recall and still be worth deploying. So the baseline's 0.870 recall is a target, not a hard bar.

- **Account for training variance.** A single training run is noisy — `gpt-4o` recall spanned 0.696–0.870 across runs, and the deployed baseline sits at the top of that spread. Gate candidates against a distribution (re-run N times, compare on mean and confidence interval) rather than one lucky draw, so "rejected" means genuinely worse rather than just unluckier. The in-loop also overfits to its own per-iteration sample (train F1 climbs while test recall drops), which larger samples or early-stopping would damp; and reasoning models can't run at `temperature=0`, so their scoring is non-deterministic and needs more repeats to compare fairly.

- **Cost from real measurement, not prediction.** Budget-derived sample sizing assumes the `gpt-4o` token profile, which doesn't transfer: the vision-token multiplier made 4o-mini ~2× *more* expensive, and reasoning-token output made gpt-5 +26% despite cheaper input. The fix is to train and evaluate every contender on equal train/test data under one budget and rank them on *real measured* per-item cost, rather than sizing each run from a baseline-derived guess. The `sample_costs` CLI (`skeet-refine/src/bin/sample_costs.rs`, built in the previous slice) is the pre-flight tool for this: run it once over a small stratified sample to get each candidate's empirical min/max/avg per-image cost before committing to long training runs — a 10-image sample would have caught every cost surprise in the phase-4 sweep.

- **Label quality.** Some gate failures may be label noise in the ~685-appraisal set rather than model error. Reviewing misclassified images and growing/cleaning the set would lift the ceiling for every candidate (and means re-capturing the frozen split to re-baseline).

- **Split scorer vs rewriter.** A previous slice used each candidate as both scorer and prompt-rewriter for simplicity. A strong rewriter producing prompts for a cheap scorer may beat one model doing both — worth testing whether the cheap models' recall collapse is the prompts or an inherent capability gap.

- **Calibration, not discrimination, was the binding ceiling.** Every cheap phase-4 candidate *ranked* images well (ROC-AUC at or above the gpt-4o baseline's 0.897) yet failed the gate because their scores sat in the wrong place on the 0–1 scale — nano overconfident (scores piled at the extremes), gpt-5 too conservative (needed a 0.22 threshold) — so none could reach 0.800 precision without dumping recall. The lever is therefore recalibrating an accepted model's scores (Platt/isotonic) or relaxing the gate from a single pinned-precision point to a (P, R) Pareto-frontier comparison — not hunting for a model with better discrimination. (The owner's precision floor of 0.800 is firm regardless, so a more lenient gate alone wouldn't change the outcome — only a candidate calibrated to high recall *at* that floor would.)

### Tasks

#### Tech-debt / bugs

##### Classify retries by HTTP status, not the blanket `Completion(_)` match

The `refine_image_resilient` wrapper's `is_transient` treats **every** `RefineError::Completion(_)` as retryable, so a permanent client error (e.g. the gpt-5 `temperature=0` HTTP 400) is retried 3× per call before falling back — wasted calls and a flood of WARN logs. Only 429, 5xx, and network errors are genuinely transient; a 4xx (other than 429) is permanent and should fail fast. The live trigger (the temperature-0/reasoning-model 400) is already resolved by the per-model `temperature_for`, so nothing is on fire — but any future permanent client error is still mis-retried.

- [ ] Preserve rig's HTTP status on the `RefineError::Completion` variant rather than stringifying the error (today the status is discarded), so retry classification has something reliable to switch on
- [ ] Rewrite `is_transient` to retry only on 429, 5xx, and network/transport errors; treat other 4xx as permanent (fail fast, no retry, no fallback churn)
- [ ] Avoid string-matching `"400"` in the error message — it's fragile; switch on the preserved status class instead
- [ ] Add unit tests: a permanent 4xx is not retried; a 429/5xx/network error is retried up to the bound

...


## Slice: Expose `skeet-appraise` as a service inside hetzner via tailscale

Use the [Tailscale Kubernetes Operator](https://tailscale.com/kb/1236/kubernetes-operator). It spins up a proxy pod per exposed resource that joins the tailnet and forwards to the backing `Service`. No public ingress, no per-service load balancer cost.

This means we can now use tailscale to expose `skeet-appraise` running as a local k8s Service inside the cluster but still have it accessible from my phone and my laptop. As part of this we need to introduce a new type of identity of appraiser based on tailscale identity.

We can do this like in Phase 3 where we run new/old alongside each other for a little while before we delete the fly.io website for `skeet-appraise`.

At end of this we can probably do a code and infra cleanup/simplification as we should no-longer need the github app / redis auth / oauth login stuff.

### Use `Ingress`, not `Service`, for identity

Of the operator's exposure modes, only [`Ingress`](https://tailscale.com/kb/1439/kubernetes-operator-cluster-ingress) injects Tailscale identity headers, which is the whole point here. Every request gets:

* `Tailscale-User-Login` — caller's login (e.g. `mike@example.com`)
* `Tailscale-User-Name` — display name
* `Tailscale-User-Profile-Pic` — profile image URL

The proxy strips incoming versions of these headers before forwarding, so they can't be spoofed from the tailnet. Anything else in-cluster reaching the backend `Service` directly could spoof them, so add a `NetworkPolicy` restricting the `Service` to only the Tailscale proxy pod.

[tailscale/tailscale#15657](https://github.com/tailscale/tailscale/issues/15657) tracks identity headers for bare `Service` resources but is open and unmoving — `Ingress` is the only option today.

### Constraints of `Ingress` mode

* HTTPS-only, port 443 only; certs auto-provisioned from Let's Encrypt.
* Requires HTTPS and MagicDNS enabled on the tailnet ([docs](https://tailscale.com/kb/1153/enabling-https)).
* Reachable only by the full MagicDNS FQDN (e.g. `bobby-appraisals-staging.<tailnet>.ts.net`) so the cert matches.
* First connection after deploy can be slow while the cert is provisioned.

### Prerequisite

OAuth client created in the Tailscale admin console for the operator — see the operator [setup section](https://tailscale.com/kb/1236/kubernetes-operator#setup). Needs **`Devices Core` + `Auth Keys` + `Services`** write scopes and the `tag:k8s-operator` tag (which must already exist in the tailnet policy file). MagicDNS + HTTPS must be enabled on the tailnet for `Ingress` mode to provision certs.

### Tasks

Spike first, then groups A–E. As in Phase 3, run the new (hetzner + tailscale) deployment alongside the old (fly + GitHub OAuth) one, verify, then cut over and clean up. Local dev keeps `--local-admin` throughout.

#### Spike (do this first): prove the Tailscale operator + `Ingress` path with a dummy service

This is the first time using Tailscale this way, so isolate the Tailscale dependency *before* touching `skeet-appraise`. Stand up the whole operator → `Ingress` → identity-header path with a throwaway workload and nothing of ours on the line. The operator and tailnet config that this sets up are **kept** and reused by group C; only the dummy workload is torn down.

* [ ] **Install the operator + enable the tailnet features** (the riskiest, least-familiar bits). Order matters — do these in sequence:
    1. **Add the operator tags to the tailnet policy file** *before* creating the OAuth client (the client must be tagged with one): `"tagOwners": { "tag:k8s-operator": [], "tag:k8s": ["tag:k8s-operator"] }`.
    2. **Create the operator OAuth client** in the admin console (see Phase 4 Prerequisite) with **`Devices Core` + `Auth Keys` + `Services`** write scopes (the `Services` scope is newer and now required), tagged `tag:k8s-operator`; store id/secret in 1Password.
    3. **Enable MagicDNS + HTTPS** on the tailnet.
    4. **`helm install` the operator** (add a `cluster-install-tailscale-operator` recipe alongside the other addon installers in `just/cluster.just`, feeding `oauth.clientId`/`oauth.clientSecret` via inline `op read` like `cluster-ghcr-secret-install` does).
* [ ] **Deploy a trivial header-echo service** — no code/build of ours: a stock multi-arch image like `traefik/whoami` (it echoes request headers, which is exactly what we need to *see* the injected identity). Give it a `Deployment` + `Service` and a tailscale-`ingressClassName` `Ingress` named e.g. `bobby-ts-spike`. Use the current Ingress shape — `spec.ingressClassName: tailscale` + `spec.defaultBackend.service` + `spec.tls.hosts` (only the **first label** of the host is used → `<label>.<tailnet>.ts.net`), *not* `rules`/`host`. **Do not set `tailscale.com/funnel: "true"`** — Funnel makes the service public *and* drops the identity headers this whole phase depends on; we want tailnet-only Serve traffic.
* [ ] **Verify the unknowns, in order:**
    * the `Ingress` provisions a Let's Encrypt cert and the service appears at `bobby-ts-spike.<tailnet>.ts.net` (first hit may be slow while the cert provisions);
    * it's reachable from **phone and laptop** over the tailnet (and *not* publicly);
    * the echo shows `Tailscale-User-Login` / `-Name` / `-Profile-Pic` populated with your identity — this is the make-or-break proof for group A;
    * a request that *sends its own* `Tailscale-User-Login` still comes back with the proxy's value (inbound copies are stripped), confirming the header can be trusted behind the ingress.
* [ ] **Prove the `NetworkPolicy`** (run it once here — group C depends on it, and NetworkPolicy enforcement is worth confirming on this hetzner-k3s cluster): restrict the dummy `Service` to the proxy pod and confirm a direct in-cluster curl is blocked while the ingress path still works — this de-risks the anti-spoofing control before group C relies on it.
* [ ] **Tear down the dummy workload** (Deployment/Service/Ingress); keep the operator, OAuth client, and MagicDNS/HTTPS settings.

#### A. Tailscale-based appraiser identity

* [ ] **Add `Appraiser::Tailscale { login }`** in `shared/src/appraiser.rs`: extend the `provider:identifier` parse/display (`tailscale:mike@example.com`), a validated `new_tailscale` constructor, and round-trip + unknown-provider tests. Mirrors the existing `GitHub`/`LocalAdmin` variants.
* [ ] **Add a header-based extractor path** for `skeet-appraise`: read `Tailscale-User-Login` from the request head and produce `Appraiser::Tailscale` (optionally surface `Tailscale-User-Name` / `-Profile-Pic` for display). This is a third source alongside the existing extensions (local-admin) and session (OAuth) paths in `AppraiserExtractor`.
* [ ] **Gate header-trust on an explicit auth-mode flag**, not header presence (the header is only trustworthy behind the Tailscale ingress; on the fly deployment it could be spoofed). Add `--auth-mode tailscale|github|local-admin` (enablement separate from config, per the rust rule). Only `tailscale` mode reads the identity headers; never auto-detect from header presence.
* [ ] **Authorization = tailnet ACLs + a required login allowlist** (decided — the allowlist is *not* optional). Tailnet ACLs gate who can reach the service; on top of that, an explicit allowlist of permitted `Tailscale-User-Login` values (the analogue of `BOBBY_ADMIN_USERS`, now holding tailscale logins/emails instead of GitHub usernames) gates who is accepted as an appraiser. Defense in depth: a tailnet identity that can reach the service but isn't on the allowlist gets `403`. The allowlist is required config in `tailscale` mode (startup fails if unset).
* [ ] **Simplify the admin guard for tailscale mode**: every request through the ingress is already identified, so there's no login/logout redirect — a missing identity header is a `403` (shouldn't happen behind the proxy). The public/admin (`is_admin`) split on the homepage collapses, since `skeet-appraise` is now a private tool; decide whether to drop it.
* [ ] **Test**: with a `Tailscale-User-Login` header in `tailscale` mode the extractor yields `Appraiser::Tailscale` and an appraisal round-trips; without it the request is denied; `github`/`local-admin` modes are unaffected.

#### B. Deploy `skeet-appraise` into the hetzner cluster

* [ ] **Build an `arm64` image** — the cluster is ARM (like `pruner`/`live-refine`), but `Dockerfile.skeet-appraise` ships `linux/amd64` for fly. During the parallel run both arches are live, so build multi-arch (`--platform linux/amd64,linux/arm64`) or add an arm64 tag; drop amd64 once fly is gone (group E).
* [ ] **k8s Deployment + Service** (`infra/k8s/skeet-appraise-deployment.yaml`): unlike the `live-refine` worker this is a long-running HTTP server, so it needs a `Service` (port 8080) fronting the Deployment. Args: `--store-path`, `--model-path`, feed-shape params, `--auth-mode tailscale`, `--bind 0.0.0.0:8080`. Env: R2 + SSE-C + OTEL only — **no GitHub/session/redis**: tailscale mode has no OAuth and no sessions, so the redis-for-sessions dependency drops out here.
* [ ] **`just/cluster.just` recipes**: `cluster-deploy-skeet-appraise`, logs, enable/disable, rollback, and add to the `cluster-*-all` aggregates (mirroring `live-refine`). Reuse the existing R2/SSE-C/OTEL `OnePasswordItem`s — no new secrets needed for the app itself.

#### C. Expose it over tailscale via the operator (`Ingress`)

The operator, OAuth client, and MagicDNS/HTTPS are already stood up and proven by the Spike — this group just applies the same, now-known-good pattern to the real service.

* [ ] **`Ingress` (not `Service`) for identity** — only `Ingress` mode injects the `Tailscale-User-*` headers (see the "Use `Ingress`" notes above). Add an `Ingress` with the tailscale `ingressClassName` for `skeet-appraise`; it provisions the cert and publishes at `bobby-appraisals-staging.<tailnet>.ts.net` (HTTPS/443 only) — exactly the path validated by the spike.
* [ ] **`NetworkPolicy` to prevent header spoofing** — the proxy strips inbound `Tailscale-User-*` headers, but anything in-cluster hitting the backend `Service` directly could forge them. Restrict the `Service` to accept traffic only from the Tailscale proxy pod (same control proven in the spike).

#### D. Parallel run + verify

* [ ] Reach `bobby-appraisals-staging.<tailnet>.ts.net` from phone and laptop over the tailnet; confirm the identity headers yield the right `Appraiser::Tailscale`, and that appraisals set/clear and the homepage + admin paging all work end-to-end (first connection may be slow while the cert provisions).
* [ ] Leave it running alongside the fly site for a while; sanity-check that appraisals made via either reach the same store and behave identically.

#### E. Cut over + cleanup

* [ ] **Decommission the fly site**: `fly apps destroy bobby-appraisals-staging`; remove `fly.appraise-staging.toml`, the `deploy_appraise_staging_*` fly recipes, and the GitHub-OAuth / session / redis secrets for that app.
* [ ] **Rip out the now-dead auth stack** from `skeet-appraise` (the cleanup the phase intro calls for): delete `auth.rs`, `auth_config.rs` (`OAuthConfig`), the `/auth/{login,callback,logout}` routes, the cot session middleware + `deadpool-redis` dep, and the `BOBBY_GITHUB_*` / `BOBBY_SESSION_SECRET` / sessions-redis config + 1Password items. Drop the `github` arm of `--auth-mode` (leaving `tailscale` + `local-admin`). Remove the GitHub OAuth app.
* [ ] **Verify post-cleanup**: `just clippy`, `just test-no-docker`; `skeet-appraise` still builds without the oauth/session deps; the relocated integration tests (now tailscale-header based) pass; drop the amd64 image build.
* [ ] Update docs (`docs/architecture.md`, any auth notes) to reflect tailscale identity replacing GitHub OAuth.

## Slice: reducing unintentional bias

### Target

The current skin-detection method in `lib.rs` (Kovac/Peer/Solina 2003 RGB rules + a YCbCr box) is biased toward lighter skin tones. Replace it with a method that performs more fairly across the Fitzpatrick scale, and add tests that would catch this kind of regression in future.

### Tasks

#### Document and demonstrate the current bias
- [ ] Write up the specific lines in `is_skin_pixel` that exclude darker skin:
    - [ ] `r <= 95.0` reject — eliminates much dark brown skin outright
    - [ ] `g <= 40.0` / `b <= 20.0` rejects — fail in shadow and on very dark skin
    - [ ] `(r - g).abs() <= 15.0` reject — absolute R−G gap shrinks at lower intensities even when the ratio is preserved
    - [ ] `max - min <= 15.0` reject — same low-intensity compression problem
    - [ ] note that the YCbCr box is the least-biased part but is ANDed with the RGB gate, so the RGB rules dominate failures
- [ ] Add failing unit tests with known dark-skin RGB samples (e.g. `(80, 50, 35)`, `(60, 40, 30)`, `(110, 75, 55)`) asserting they should be classified as skin — these should fail against the current implementation and pass against the replacement
- [ ] Assemble a small evaluation set of face images spanning Fitzpatrick I–VI and measure per-bucket true-positive rate before and after the change

#### Pick a less-biased method
- [ ] Evaluate options in roughly increasing order of effort:
    - [ ] **CbCr-only elliptical region** (Hsu, Abdel-Mottaleb & Jain, 2002) — drop the RGB gate entirely, fit an ellipse in CbCr space rather than an axis-aligned box. Small code change, big fairness improvement.
    - [ ] **HSV or normalised-rgb thresholds** — hue ≈ [0°, 50°] with moderate saturation and *any* value; removes the luminance dependency that hurts dark skin
    - [ ] **Jones & Rehg statistical skin model** (1999) — Bayesian histogram trained on a large diverse pixel set, runtime is a 3D lookup table, still the standard classical baseline
    - [ ] **Modern ML model trained on a diverse dataset** — anything evaluated on Fitzpatrick 17k or trained on FSD/ECU/Pratheepan; highest accuracy, adds a dependency

#### Rust ecosystem options
- [ ] **Pure-Rust / classical (no new heavy deps).** There is no dedicated "less-biased skin detector" crate on crates.io — closest neighbours are face-detection crates, not skin segmentation. So this path is hand-rolled on top of the existing `image` crate:
    - [ ] Implement a CbCr-ellipse or HSV test directly in `lib.rs`
    - [ ] Optionally fit a Jones-and-Rehg histogram offline against the [UCI Skin Segmentation dataset](https://archive.ics.uci.edu/ml/datasets/skin+segmentation) (built from face images "of diversity of age, gender, and race") and ship the resulting lookup table as a `.bin` in the repo
- [ ] **ML model via ONNX.** The standard route for running pretrained vision models from Rust is the [`ort`](https://crates.io/crates/ort) crate (ONNX Runtime bindings). [`rust-faces`](https://crates.io/crates/rust-faces) is a good template for how to wire an ONNX model into a Rust API similar to our `detect_skin` signature.
- [ ] **Candidate model:** [samhaswon/skin_segmentation](https://github.com/samhaswon/skin_segmentation) on GitHub — a benchmark/training repo with ONNX exports of several skin-segmentation models (BiRefNet, U²-Net variants, etc.). The author explicitly built the training set "to maximize diversity of scene, lighting, and skin appearance" with augmentations designed so the model isn't dependent on lighting or camera settings. Caveat: the heaviest BiRefNet variant uses ~40 GB RAM through onnxruntime, so pick one of the smaller CNN models.
- [ ] **Background reading** for evaluation methodology and fairness framing:
    - [ ] [Fitzpatrick 17k](https://github.com/mattgroh/fitzpatrick17k) (Groh et al., 2021) — standard fairness benchmark
    - [ ] [Bencevic et al. (2024)](https://www.sciencedirect.com/science/article/pii/S0169260724000403) — quantifies the same bias pattern across U-Net-based skin segmentation models

#### Recommended path
- [ ] **Step 1 — cheap win.** Replace the RGB+CbCr-box rules with either a CbCr ellipse or a Jones-and-Rehg histogram trained on the UCI dataset. Pure Rust, no new heavy deps, almost certainly closes most of the gap. Add the dark-skin unit tests above so the improvement is visible.
- [ ] **Step 2 — only if step 1 isn't good enough.** Add `ort` and load one of the smaller models from samhaswon/skin_segmentation behind an optional feature flag (`features = ["ml"]`), keeping the classical path as the default so the binary stays small.

#### Guardrails
- [ ] Keep the per-Fitzpatrick-bucket eval as a checked-in test or bench so future changes can't silently regress fairness
- [ ] Update the doc-comment on `detect_skin` to honestly describe what the method does and its known limitations

## Slice: replay-based regression testing

### Target

Catch performance and cost regressions before they reach production. The motivating incident is the 26 Apr R2 Class A blowup (see `docs/current-slice.md`): a one-line change in `bc59e99` 10×'d the pruner's LIST rate against R2 and was only caught after deploy by reading Grafana graphs. The shape we want: capture real input traffic for a few minutes, replay the pipeline against a local backend, snapshot the resulting OTel counters, fail the test when any counter moves outside an expected band.

This is **not** deterministic simulation testing in the FoundationDB/TigerBeetle sense — no fake runtime, no concurrency exploration, no fault injection. Just: capture → replay → snapshot → diff.

What it catches:
* R2 op-count regressions (the 26 Apr incident)
* Queue-depth regressions (assert p99 of the depth gauge stays low)
* Throughput cliffs (events processed per simulated minute)
* Anything else expressible as an OTel metric

Out of scope:
* LLM cost regressions
* Concurrency bugs that would need a real DST framework
* Behaviours that only emerge under load longer than the fixture

### Tasks

#### Phase 1: replay infrastructure for pruner

* [ ] make the firehose source pluggable: `firehose::connect` currently returns a concrete `JetstreamReceiver`. Refactor the pipeline to accept any `Stream<Item = JetstreamEvent>` so a JSONL-backed source can drop in
* [ ] add a `capture` CLI that, for a given `--duration`:
    * records firehose events to `tests/fixtures/<name>/firehose.jsonl`
    * snapshots the live R2 store to a tarball at `tests/fixtures/<name>/store.tar`
    * records image HTTP GETs to `tests/fixtures/<name>/images/`
    * keep fixtures small enough to commit; if they grow past a few MB, move to git-lfs or an R2 fixtures bucket
* [ ] write a `replay_pruner` integration test that:
    * extracts the store tarball into a tempdir
    * opens the store via `file://` (existing `StoreArgs::open_store` path) wrapped in `R2MetricsWrapper` — the wrapper produces cost-equivalent counts against local disk
    * serves recorded image responses via [`wiremock`](https://github.com/LukeMathWalker/wiremock-rs)
    * drives the JSONL stream into the pluggable firehose source
    * runs until the stream ends, then `force_flush()`s the OTel meter and serialises all counters/gauges (the `InMemoryMetricExporter` pattern in `store_metrics.rs` already shows the shape)
* [ ] assert via a checked-in `expected-metrics.json` with explicit per-counter ranges (e.g. `r2.operations{operation="list"}: 60..120`). Prefer this over `insta`-style auto-blessing — clearer failure messages, no risk of someone blessing a 10× regression by reflex
* [ ] wire into `just test` so it runs in CI

#### Phase 2: extend to live-refine

* [ ] record OpenAI API responses keyed by request hash for the fixture window (wiremock or [`rvcr`](https://github.com/ChorusOne/rvcr))
* [ ] write `replay_live_refine` mirroring the pruner test, asserting both R2 ops and OpenAI request counts

#### Maintenance

* [ ] document how and when to refresh fixtures and the expected-metrics baseline — only when fixtures no longer represent production (e.g. firehose schema change, store schema change), not on every behaviour change
