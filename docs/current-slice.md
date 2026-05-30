# Current Slice: Split out `skeet-feed`/`skeet-appraise`/`skeet-publish`

### Target

I want to get to the following different division of responsibilities:
* `skeet-feed`:
    * lives at `bobby-staging.houseofmoran.io`
    * handles:
        * bluesky feed
        * public website listing skeets ordered by recency and filtered by band >= MedHigh; this is a much simpler page than today's homepage (the current rich homepage moves to `skeet-appraise`)
    * bias is towards simplicity, reliability and speed (latency/cachability)
* `skeet-appraise`:
    * lives at `bobby-appraisals-staging` (the eventual MagicDNS FQDN `bobby-appraisals-staging.<tailnet>.ts.net`) within the hetzner cluster, accessible via tailscale
    * handles:
        * showing current status and editable controls (appraisals) for:
            * what is currently live as the feed (this is effectively the current `skeet-feed` homepage, moved here)
            * what has been found by the pruner and refiner for each skeet and associated images
        * manual appraisals (assigning High/MedHigh/MedLow/Low)
    * bias is towards ease-of-use and quick interactive updates
* `skeet-publish`:
    * runs in hetnzer k8s cluster like `live-refine` looking for changes to dependent tables
    * handles:
        * watching for changes in what skeets / images have been found and scored by a model as well as what has been appraised
        * determining what needs to be published as the feed; this is the canonical single place we decide this
        * this is where we apply the "ordered by recency and filtered by band >= MedHigh" from above i.e. the `skeet-feed` just blindly accepts the ordering specified by the publisher
        * deriving the image URL for each published image (the redis Feed stores `image-url:skeet-id` pairs)
* `skeet-refine` / `skeet-prune` stay as-is

The parts are related as follows by introducing a new redis table in upstash that sits between publisher and feed. The publisher writes `image-url:skeet-id` pairs to this table (it derives the image URL). The Bluesky feed reads the pairs and extracts a unique, ordered list of skeet-ids; the image URLs are used (from Phase 5 onwards) to render the public image grid:

```mermaid
architecture-beta
    group redis(cloud)[Upstash]
    group fly(cloud)[Fly]
    group r2(cloud)[R2]
    group hetzner(cloud)[Hetzner Cluster]

    service feed-table(database)[Feed] in redis
    service pruned-table(database)[Pruned] in r2
    service refined-table(database)[Refined] in r2
    service appraised-table(database)[Appraised] in r2
    service feed(server)[Bluesky Feed] in fly
    service publisher(server)[Publisher] in hetzner

    junction publisherJunction in r2

    feed:R -- L:feed-table
    publisher:T --> B:feed-table
    pruned-table:T -- B:publisherJunction
    refined-table:R -- L:publisherJunction
    appraised-table:B -- T:publisherJunction
    publisherJunction:R --> L:publisher
```

### Bugs / Refactors

#### Scope each Dockerfile to its crate with `-p`

##### Core, ahead of other work

* [x] audit each Dockerfile and map it to the single crate it ships, then scope both the chef cook and the final build to that crate:
    1. `Dockerfile.skeet-feed` → `-p skeet-feed` (bin `skeet-feed`)
    2. `Dockerfile.pruner` → `-p skeet-prune` (bin `pruner`)
    3. `Dockerfile.live-refine` → `-p skeet-refine` (bin `live-refine`)
    4. (no `Dockerfile.compact`/`Dockerfile.bench-firehose` exist; the other Dockerfiles are:)
        - `Dockerfile.optimise` → `-p skeet-store` (bin `optimise`)
        - `Dockerfile.cloudflare-exporter` → `-p cloudflare-exporter` (bins `sync_operations`, `sync_storage`)
        - `Dockerfile.openai-exporter` → `-p openai-exporter` (bin `sync_costs`)
* [x] in the builder stage, replace the workspace-wide cook with a scoped cook so only the crate's dependency subtree compiles:
    * `cargo chef cook --release -p <crate> --recipe-path recipe.json`
    * leave `cargo chef prepare` running over the whole workspace — the recipe is deps-only and can stay shared
* [x] replace the final `cargo build` with `cargo build --release -p <crate> --bin <bin>` so each image stops compiling sibling binaries. Kept `--bin` because several crates have many binaries (`skeet-prune` has 6, `skeet-refine` has 5); a bare `-p <crate>` would compile all of them.
* [x] **TLS / `deadpool-redis`:** the note originally said "`live-refine` / `skeet-refine`", but `skeet-refine` has no redis/`cot` dependency at all. The crate that uses `cot` + `deadpool-redis` + TLS-to-Upstash is **`skeet-feed`**, and it declares both directly, so the scoped `-p skeet-feed` cook keeps them in the same dependency subtree and the feature-unification HACK still applies. No `Cargo.toml` change was needed. *(Still needs Docker-build verification that TLS to Upstash works in the built `skeet-feed` image.)*

##### Discover / improve as we do this slice

* [ ] verify caching actually improves: with deps unchanged, touch only `<crate>/src/main.rs`, rebuild, and confirm the cook/deps layer is reused (no dependency recompile). This is the direct test of whether `-p` + chef fixes the "recompiles everything" symptom for source-only changes.
* [ ] (optional) now that each image cooks its own copy of the shared deps (`tokio`, `reqwest`, `image`, tracing/otel, `shared`), decide whether to dedup across images: BuildKit cache mounts on `target/` + the cargo registry, or `--cache-from` the previous git-hash-tagged image (ties into Q2 above on git-hash layer caching)
* [ ] apply this same `-p` pattern when adding the `skeet-appraise` and `skeet-publish` Dockerfiles (Phases 1 and 3) rather than cloning a workspace-wide build

#### Make use of git-hash?

* [ ] now that docker images are built and named based on git-hash, can we exploit that for a more exact caching of layers?

#### Deny `expect()` as well

`expect()` is probably as bad an idea in main code as `unwrap()` so deny that as well, and instead prefer explicit Result+Err, unless in tests.

* [ ] similar to `unwrap_used = "deny"` and `allow-unwrap-in-tests = true` do the same for expect, and fix all related issues
* [ ] add a note about this to @rust.md like we do for `unwrap`

### Phases

We'll do this in phases, with a working system at each step

#### Phase 1: Split out `skeet-appraise` as a standalone website

Even though we want to ultimately make this run within the hetzner cluster and be accessible over tailscale, initially we'll introduce a new fly.io website at `bobby-appraisals-staging.houseofmoran.io`. 

This can effectively copy/clone setup we already have for `bobby-staging.houseofmoran.io` as we are largely splitting out existing code.

Tasks:
...

#### Phase 2: Split out `skeet-publish` as a library

This is not introducing a new service, but instead is factoring out the code already in `skeet-feed` which is to do with caching and generating a feed to instead live in `skeet-publish` crate. This should live behind a trait which abstracts away as much detail as possible. The `skeet-feed` should depend only on this trait.

Tasks:
...

#### Phase 3: Turn `skeet-publish` into a service

This is where we introduce a new redis `feed` storage to act as the publishing destination which links `skeet-feed` and `skeet-publish`. we can do this in steps:
1. Create a new redis list in upstash called `feeds` which will contain a list of `image-url:skeet-id` pairs (the publisher derives the image URL), which represent the images which have been allowed through. `skeet-feed` reads these pairs and extracts a unique, ordered list of skeet-ids for the Bluesky feed
2. Create a new service which works like `live-refine` except it monitors and periodically recalculates the pairs (based on same logic as was in `skeet-feed` but has now been moved to this library), and then publishes this to the redis list. Deploy this to hetzner and leave running for an afternoon (verify manually that redis list makes sense).
3. Update `skeet-feed` to be configurable (via config flag) to either continue using the library implementation or reading from redis (using different implementations of same trait). Deploy this to staging with it told to use the redis input. Deploy and leave running for an afternoon and manually verify it makes sense.
4. If all good, remove implementation of trait that does live calculation and instead rely only on redis implementation.
5. Switch `skeet-feed` to be a suspendable service (see below)

##### `skeet-feed` as a suspendable Fly service: things to know

- **Eligibility:** ≤ 2 GB RAM, no swap, no GPU, machine updated since June 2024.
- **Redis connection dies on resume.** Upstash's idle timeout fires during suspension; local socket doesn't notice. Need a pool that validates before use, or retry-on-failure that reconnects + re-auths.
- **Same for any other long-lived outbound HTTP pools**
- **Every deploy invalidates the snapshot** — first request after deploy is a real cold start, not a resume. Keep the cold-start path fast (lazy-load from Redis, don't preload).
- **Tune `soft_limit`** on the HTTP service in `fly.toml` — controls how aggressively the proxy suspends. Default is too high for low-traffic staging.
- **Timers pause during suspend** and clock can lag a few seconds on resume. Use wall-clock for anything time-sensitive; don't trust `tokio::time::interval` cadence as real-time.
- **Logs and metric pushes can drop** across the suspend boundary. Don't alert on metric absence.
- **Keep health checks shallow**, or have them go through the same retry path as real requests.

Tasks:
...

#### Phase 4: Expose `skeet-appraise` as a service inside hetzner via tailscale

Use the [Tailscale Kubernetes Operator](https://tailscale.com/kb/1236/kubernetes-operator). It spins up a proxy pod per exposed resource that joins the tailnet and forwards to the backing `Service`. No public ingress, no per-service load balancer cost.

This means we can now use tailscale to expose `skeet-appraise` running as a local k8s Service inside the cluster but still have it accessible from my phone and my laptop. As part of this we need to introduce a new type of identity of appraiser based on tailscale identity.

We can do this like in Phase 3 where we run new/old alongside each other for a little while before we delete the fly.io website for `skeet-appraise`.

At end of this we can probably do a code and infra cleanup/simplification as we should no-longer need the github app / redis auth / oauth login stuff.

##### Use `Ingress`, not `Service`, for identity

Of the operator's exposure modes, only [`Ingress`](https://tailscale.com/kb/1439/kubernetes-operator-cluster-ingress) injects Tailscale identity headers, which is the whole point here. Every request gets:

* `Tailscale-User-Login` — caller's login (e.g. `mike@example.com`)
* `Tailscale-User-Name` — display name
* `Tailscale-User-Profile-Pic` — profile image URL

The proxy strips incoming versions of these headers before forwarding, so they can't be spoofed from the tailnet. Anything else in-cluster reaching the backend `Service` directly could spoof them, so add a `NetworkPolicy` restricting the `Service` to only the Tailscale proxy pod.

[tailscale/tailscale#15657](https://github.com/tailscale/tailscale/issues/15657) tracks identity headers for bare `Service` resources but is open and unmoving — `Ingress` is the only option today.

##### Constraints of `Ingress` mode

* HTTPS-only, port 443 only; certs auto-provisioned from Let's Encrypt.
* Requires HTTPS and MagicDNS enabled on the tailnet ([docs](https://tailscale.com/kb/1153/enabling-https)).
* Reachable only by the full MagicDNS FQDN (e.g. `bobby-appraisals-staging.<tailnet>.ts.net`) so the cert matches.
* First connection after deploy can be slow while the cert is provisioned.

##### Prerequisite

OAuth client created in the Tailscale admin console for the operator — see the operator [setup section](https://tailscale.com/kb/1236/kubernetes-operator#setup).

##### Tasks
...

#### Phase 5: turn `skeet-feed` homepage into a simple-but-nice list of images

What I am envisaging here is a pinterest-style layout using css-grid. This should show all images seens in past week, and a click on each goes to the skeet. This may involve extending the publisher to publish a larger list of all images seen in past week (not just past couple of days that show in feed).

This should be as server-rendered as possible, with associated cache headers on images and similar to maximise cache-ability.

Tasks:
...
