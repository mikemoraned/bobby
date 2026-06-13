# Current Slice: safe-ish production / staging separation

### Target

A separation between production and staging setups to avoid accidental breakage.

This is the **foundation** for moving to a 1.0 public `bobby.houseofmoran.io` feed: I can't safely run a stable production feed alongside per-worktree dev until I've defined how the two stay out of each other's way. So this slice does *only* that separation.

What I want:

* Ideally a setup where there is a `production` environment (perhaps represented as a namespace in k8s) which contains the stable components I don't want to break. Then, have a per-worktree staging setup where I can create a new worktree and, if I want to, have a unique set of components for that worktree.
* I don't want to go overkill with this: it's just something that allows me to continue dev on code without breaking production.
* However, I don't want to duplicate components for every worktree. I'd like to have something like:
    * services and jobs which can share backend data stores like R2 across envs. So, there is not a "staging" R2 store or Redis, but instead, where possible, we use versioning of tables and collections to give safe-ish separation. I say safe-ish as there is still a possibility that staging components could interfere with prod components. However, having a fully separate env for each staging setup is costly and also means any staging setup starts from scratch with data, which is likely not useful for quick iteration in the future.
    * this versioning approach should extend into models; so we probably should have a `production` label and a label per-contender.
    * this also means we should have a more explicit "promotion" process where we use model or k8s labels, or similar, to promote something developed on a branch into a main prod version. This should be supported by local cli lifecycle commands and/or third-party tools.
    * build and deployment can continue to be done locally on my laptop.

The way the versioning would generally work is that we have a split between across-infra versioning and within-infra versionings i.e.
* we have an R2 `encrypted-store` which is the main version which is shared across prod and any staging setup
  * within this (lancedb) store, we name tables by `<type>_<version>`, so if type = `images`, version = `v3` -> then table is `images_v3`; if something needs to make a backwards-incompatible change to a type or introduce a new type then it should create a new table
  * then, within each table, individual columns may support more fine-grained versioning, like the model version or ImageId format string. this means that prod and staging components could still be writing data which only they understand into different rows of the same table. This is fine as long as we write the software so that it only selects for what it knows, or discards anything it sees which it doesn't understand.
  * if a slice needed to make a change that is more fundamental (e.g. we are not using lancedb anymore, but we still use R2) then it should create an entirely new top-level store e.g. `some-other-store`
* similarly, we should have a shared redis store in upstash which is named `bobby-v1`
  * within this redis store, items are prefixed with a version string e.g. like `v3-recency-48h` so format is effectively `<version>-<type>`. A slice that needs to introduce a new type or version can do so without affecting prod.
  * note that there is no easily equivalent of more fine-grained versioning here. An example of why is the JSON data we write to something like `v3-recency-48h`: if we are extending this to add a new JSON key, we can make prod handle that whilst staging is writing new stuff by telling serde to ignore unknown new keys. However, of prod can still write the old setup then it doesn't work as staging needs those new keys: prod could write a JSON value which isn't understood by the staging version as it is missing the new keys needed. IIRC, I think this is similar to the problem of covariance of reads and contravariance of writes in functions?

### Approach: lightweight, conventions-first

Much of this already exists in some form — there's a staging/prod split at the deployment layer (`bobby-staging.houseofmoran.io`), table versioning (`images_v1/v2/v3`), a model registry carrying `decision_threshold` + `ModelVersion`, and per-`(order, limit)` redis keys. The job here is mostly to **make the conventions I already follow explicit and systematic**, plus add a lightweight promotion path — *not* to build a generic multi-environment framework.

#### Decide the acceptable safety level first

The stated goal is "avoid accidental breakage," but sharing R2/Redis via version-prefixing keeps a real possibility that staging interferes with prod (I've already flagged this as "safe-ish"). So before building anything, decide and write down what level of safety is actually required, e.g.:

* which stores/tables/keys are **prod-only** and must never be written by a dev worktree;
* where version-prefixing is sufficient ("safe-ish") and where a hard separation is genuinely warranted;
* the failure modes I'm explicitly accepting in exchange for shared data and quick iteration.

This decision drives everything below.

### Tasks

* [ ] Decide and document the acceptable safety level (above) — the contract for what prod guarantees and what staging may touch.
* [ ] Write down the existing naming conventions as the separation mechanism: table version suffixes, redis key prefixes/namespacing, and model labels (`production` vs per-contender). Capture these in `docs/` so they're a deliberate convention, not folklore.
* [ ] Extend the model versioning with an explicit `production` label plus a label per-contender, so what's serving prod is unambiguous.
* [ ] Define the promotion process: a lightweight checklist plus minimal local CLI lifecycle command(s) to promote a model/component from a branch to the prod version (using model and/or k8s labels). Third-party tooling only if it's clearly simpler than a small CLI.
* [ ] Represent the `production` environment in k8s (e.g. a namespace) holding the stable components, with per-worktree staging able to run its own components when wanted — without duplicating the shared backend stores.
* [ ] Keep build and deployment local on my laptop; capture any new invocations in the Justfile.

