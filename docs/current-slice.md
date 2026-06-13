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

### Approach: mostly already built — write it down, then close the gaps

The versioning model above is, to a large extent, **already the implemented convention** — verified in the code:

* **Lance tables already follow `<type>_<version>`:** `images_v6`, `images_score_v2`, `manual_skeet_appraisal_v1`, `manual_image_appraisal_v1` (`skeet-store/src/schema.rs`). (The `v3` in the example above is illustrative; the live images table is `v6`.)
* **Redis keys already follow `<version>-<type>`:** `PublishedList::name()` is `{SCHEMA_VERSION}-{order}-{limit}` → `v3-recency-48h`, with `SCHEMA_VERSION = "v3"` the single source of truth in `skeet-publish`. Its doc-comment already records the collision-avoidance rationale.
* **Scores already carry a `model_version` discriminator column** → contender scores coexist with prod scores as distinct rows in the shared `images_score_v2`.

So the data-plane half of this slice is largely a **documentation + audit** job, not new construction. The genuinely new work is: the model `production` label + promotion path, the compute-side (k8s) isolation, and resolving one real gap the model doesn't yet cover (below).

#### The load-bearing principle

What actually makes "shared stores, safe-ish" work: **only stand up a staging instance of a writer when you're changing it — and changing it means a new version (table or key), which auto-isolates it from prod. Components you aren't changing aren't duplicated; staging reuses prod's data/output.** The residual risk only materialises if a staging *writer* runs at the *same* version as prod.

#### The coexistence rule (covariance/contravariance)

A **reader is covariant** — tolerant of supersets, so making readers *ignore unknown fields / treat new fields as optional* lets you add fields while old writers still run. A **writer is contravariant** — any field a reader *requires* must be produced by *every* writer feeding it. So a field that's required forces a flag-day across all writers. Since a prod/staging split deliberately does **not** upgrade prod's writer in lockstep, you can't evolve a *shared* value's required shape — you bump the version to get a fresh container.

The lance-vs-redis difference is just **where the discriminator lives**:
* **lance** — a *column inside* the shared table (fine-grained): two shapes coexist as different rows, the reader filters `WHERE version ∈ {known}`.
* **redis** — only the *key name* (coarse): the value is monolithic, there's nowhere inside it to put a discriminator a reader can filter on, so any incompatible shape change = a new key (bump `SCHEMA_VERSION`).

That's exactly *why* there's "no fine-grained versioning" for redis — it's not an oversight, it's the absence of a sub-key discriminator.

#### The gap the model doesn't yet cover: row-clobber where the key has no discriminator

"Writes into different rows of the same table" is only safe **where the primary key carries a discriminator**:

* `images_score_v2`, keyed by `(image_id, model_version)` → a contender writes *new* rows → safe. ✓
* `manual_*_v1`, keyed by `(id, appraiser)` → appraisals discriminated by appraiser; sharing them across prod/staging is *desirable* (they're the label set). ✓
* `images_v6`, keyed by `image_id` (content hash) **alone, no discriminator** → a staging **pruner** that changes an existing column's *value* (zone, annotation, detected_text) for an image prod also sees **overwrites prod's row in place**. Bumping to `images_v7` "fixes" it but is heavy and throws away the shared data that made sharing worthwhile.

This is the real "acceptable safety level" decision for this slice. **Recommended policy: never run a staging pruner against the shared store.** Iterate the pruner offline (slice-16 `eval` harness + a local `file://` store); only the *promoted* pruner writes `images_vN`. The expensive iteration loops (refine, feed/publish) read the images table read-only, so this costs nothing in practice. Reject the heavier alternatives (add an owner discriminator to the images key; bump the table per change) unless offline iteration proves insufficient.

### Tasks

* [ ] **Decide & record the pruner-vs-shared-images policy** (above) — the headline safety-level decision. Note the per-table coexistence story (`images_score_v2` discriminated; `manual_*` by appraiser; `images_v6` not, hence the policy).
* [ ] **Write the canonical versioning doc** (e.g. `docs/versioning.md`): the across-infra vs within-infra split from the Target, the coexistence rule and covariance/contravariance reasoning, and the current verified names (`images_v6`, `images_score_v2`, `v3-<order>-<limit>` / `SCHEMA_VERSION`, the `model_version` discriminator). This documents what's already true plus the rules for changing it.
* [ ] **Audit conformance + the filter-on-read / ignore-unknown invariant.** Confirm every reader (a) derives redis keys from the single `SCHEMA_VERSION` source rather than hardcoding a prefix, and (b) selects only the versions/models it understands and discards/ignores unknown rows and JSON keys (`#[serde(default)]` / deny-or-ignore-unknown). Fix any reader that assumes a fixed schema or does `SELECT *`-and-trust.
* [ ] **Model `production` label + per-contender labels.** Extend the model registry so exactly one `ModelVersion` is labelled `production`; the feed/publisher reads scores `WHERE model_version = <the production label>`. Contenders write under their own label into the shared `images_score_v2` and never affect prod.
* [ ] **Promotion = repoint the label** (no data migration): a small local CLI command to set/show the `production` label, and — for compute — flip the prod k8s deployment to the corresponding image. Plus a short checklist for the human steps.
* [ ] **Compute isolation (k8s) via a dedicated `production` namespace**, separate from the data-plane versioning. Move the stable components into a `production` namespace (its own Secrets / `OnePasswordItem`s, OTel `deployment.environment=production`), so a staging deployment can't restart, scale, or mis-secret prod pods. A worktree that needs to run a *changed* component runs it in its own namespace (distinct names/labels), sharing the backend stores. Unchanged components are not duplicated — staging reuses prod's data/output. Keep the namespace move minimal: don't replicate stores, don't add per-worktree infra beyond the changed component.
* [ ] **Build & deploy stay local;** capture any new invocations in the Justfile.
