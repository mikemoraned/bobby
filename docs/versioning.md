# Versioning & production / staging separation

This is the canonical description of how Bobby keeps a stable **production**
deployment and any number of per-worktree **staging** deployments out of each
other's way *without* duplicating the backend data stores. It documents the
conventions already implemented in the code plus the rules for changing them.

## The shape of the problem

Production and staging share the same backend stores — there is **no** separate
staging R2 bucket or Redis. Isolation comes from **versioning the names of
things inside those shared stores**, not from standing up parallel
infrastructure. Two levels:

- **across-infra versioning** — the top-level store identity. One R2
  `encrypted-store` (a LanceDB store) shared by everyone; one Upstash Redis
  `bobby-v1`. A change so fundamental that the store type itself changes (e.g.
  no longer LanceDB) gets an entirely new top-level store (`some-other-store`),
  never an in-place migration of the shared one.
- **within-infra versioning** — how items inside a shared store are named so
  incompatible shapes coexist instead of clobbering each other (below).

### The load-bearing principle

Only stand up a staging instance of a **writer** when you're *changing* it — and
changing it means a new version (a new table or a new key), which auto-isolates
it from production. Components you aren't changing are not duplicated; staging
reuses production's data and output. The residual risk only materialises if a
staging *writer* runs at the *same* version as production.

## Within-infra versioning

### LanceDB tables: `<type>_<version>`

Tables are named `<type>_<version>`. A backwards-incompatible change to a type,
or a brand-new type, creates a new table.

Within a table, individual **columns** can carry finer-grained versioning (e.g.
`model_version`, the `ImageId` format prefix). This lets production and staging
write rows only they understand into the same table, *as long as* readers select
only what they know and ignore the rest (see the coexistence rule).

### Redis keys: `<version>-<type>`

Redis list keys are `{SCHEMA_VERSION}-{order}-{limit}`, e.g. `v3-recency-48h`.
`SCHEMA_VERSION` (`skeet-publish/src/published.rs`) is the single source of truth
and is bumped whenever the stored `PublishedImage` JSON changes incompatibly.
The key is always derived through `PublishedList::name()` — both the writer
(publisher) and the reader (`skeet-feed`, via `skeet-publish/src/source.rs`)
build it the same way, so neither hardcodes a prefix.

There is **no** finer-grained versioning inside a redis value: the value is a
monolithic JSON blob with nowhere to put a discriminator a reader could filter
on. So any incompatible value-shape change is a new key (a `SCHEMA_VERSION`
bump). That is not an oversight — it's the absence of a sub-key discriminator
(below).

## The coexistence rule (covariance / contravariance)

- A **reader is covariant** — tolerant of supersets. Making readers *ignore
  unknown fields / treat new fields as optional* lets you add fields while old
  writers still run.
- A **writer is contravariant** — any field a reader *requires* must be produced
  by *every* writer feeding it. So adding a newly-required field means you must
  update every writer to produce it before any reader can rely on it, and all
  those writers have to be redeployed together in one coordinated cutover (you
  can't have some old writers still omitting the field).

Because a prod/staging split deliberately does **not** upgrade production's
writer in lockstep, you cannot evolve a *shared* value's required shape. To get a
fresh container for the new shape you bump the version.

The lance-vs-redis difference is just **where the discriminator lives**:

- **lance** — a *column inside* the shared table (fine-grained). Two shapes
  coexist as different rows; the reader filters `WHERE version ∈ {known}`.
- **redis** — only the *key name* (coarse). The value is monolithic, so any
  incompatible shape change means a new key.

### Conformance (audited 2026-06-13)

A snapshot of what was true at the audit; the conventions above are the
long-lived part. Re-derive the table list from `skeet-store/src/schema.rs` rather
than trusting this if it has aged.

Current live tables (`skeet-store/src/schema.rs`):

| Table | Key | Notes |
|-------|-----|-------|
| `images_v6` | `image_id` (content hash), **no discriminator** | see pruner policy below |
| `images_score_v2` | `(image_id, model_version)` | scores discriminated by `model_version` |
| `manual_skeet_appraisal_v1` | `(skeet_id, appraiser)` | discriminated by appraiser |
| `manual_image_appraisal_v1` | `(image_id, appraiser)` | discriminated by appraiser |

- Redis keys are derived from `SCHEMA_VERSION` via `PublishedList`, never
  hardcoded (the only literal `v3-...` strings are test assertions).
- No type deserialised from a shared store uses `#[serde(deny_unknown_fields)]`,
  so readers ignore unknown JSON keys (covariant reads). New optional fields use
  `Option<T>` / `#[serde(default)]`.
- Score reads filter to the known model-version set (below) rather than
  `SELECT *`-and-trust; image reads select named columns.

## The pruner-vs-shared-images policy (the headline safety decision)

"Writes into different rows of the same table" is only safe **where the primary
key carries a discriminator**:

- `images_score_v2` keyed by `(image_id, model_version)` → a contender writes
  *new* rows → safe.
- `manual_*_v1` keyed by `(id, appraiser)` → discriminated by appraiser; sharing
  across prod/staging is *desirable* (it's the shared label set).
- `images_v6` keyed by `image_id` (content hash) **alone, no discriminator** → a
  staging **pruner** that changes an existing column's *value* (zone,
  annotation, detected_text) for an image production also sees would **overwrite
  production's row in place**.

**Policy: never run a staging pruner against the shared store.** Iterate the
pruner offline (`eval` harness + a local `file://` store); only the
*promoted* pruner writes `images_vN`. The expensive iteration loops (refine,
feed/publish) read the images table read-only, so this costs nothing in
practice. The heavier alternatives (an owner discriminator on the images key; a
table bump per change) are rejected unless offline iteration proves
insufficient.

The "no discriminator" on `images_v6` is a property of the *current* schema, not
a fundamental limit. We could make the table discriminable the same way
`images_score_v2` is: have the pruner write its own version (pruner code version
+ prune-config/model version) into a column, make it part of the key, and have
readers filter `WHERE pruner_version ∈ {known}` — exactly the writer-label /
reader-known-set pattern used for refine model scores. Then a staging pruner
would write *new* rows instead of overwriting production's, and the
"never run a staging pruner against the shared store" policy could relax.
**We are not doing this yet** — offline iteration is sufficient today, and adding
the column means an `images_v7` bump. It's the escape hatch if offline iteration
ever proves too slow.

**Guardrail (enforced in code, not just prose):** `pruner` takes
`--allow-shared-store-write` (default off) and refuses to start when its store is
remote (`s3://`, which includes R2) without it (`skeet-prune/src/bin/pruner.rs`,
using the pure `StoreArgs::is_remote()` inspector). Production's
`pruner-deployment.yaml` passes the flag; a staging worktree pointed at a local
store needs nothing.

## Model scores: writers and readers are asymmetric

The `model_version` discriminator on `images_score_v2` is read and written under
*different* rules — the covariance/contravariance rule applied to the
discriminator column, with `config/refine.toml`'s registry as the "known set".

- **Writers** (refine / live-refine, `RefineModels::by_label(&Label::production())`)
  always write under the **`production` label** — exactly one model version. This
  keeps staging from polluting freshly-written production scores.
- **Readers** (the publisher's feed-visibility path) must tolerate scores written
  by *older or other registered* models, so they filter
  `WHERE model_version ∈ (every version in refine.toml)` —
  `RefineModels::versions()` — **not** `= production`. Historical rows from a
  previously-promoted version don't vanish when the label moves; an unregistered
  staging/experimental `model_version` is silently discarded. That discard is the
  safety property, applied at read time in
  `SkeetStore::list_scored_summaries_published_since` /
  `list_scored_summaries_by_score` (the publisher passes the known set in).
- Choosing *which* known score drives ranking (e.g. prefer the production
  version) is a separate ranking concern, not this safety filter.

The single-row/admin inspectors (`get_score`, `list_scores_for_ids`, used by the
appraisal UI) intentionally show whatever is stored, including unregistered
versions — they are inspection tools, not the feed path.

Why this is exactly-one-production by construction: `RefineModels.labels` is a
`HashMap<Label, ModelVersion>`, so `production` can point at exactly one version
(`shared/src/refine_model.rs`). Load-time validation rejects duplicate versions
and labels pointing at unregistered versions. Per-contender labels are just other
entries in that map.

## Promotion = repoint the label (no data migration)

Promotion moves which registered model is `production`; it does **not** migrate
any data. The `promote` bin (`skeet-refine`) does this:

```
just refine-promote-show          # list labels and registered models
just refine-promote-set <version> # repoint `production` at a registered version
```

`set` validates the target version is registered (else `UnknownLabelVersion`) and
rewrites `config/refine.toml` via `RefineModels::save`. Readers immediately widen
to include the newly-promoted version because they filter on *all* registered
versions, and a prior production version's historical scores keep resolving for
the same reason.

### Manual checklist: flipping the k8s image after a promotion

The CLI does **not** touch k8s. After `just refine-promote-set <version>`:

1. Commit the `config/refine.toml` change.
2. Build & push the images for the affected writers (`live-refine`, and
   `pruner` if its config changed) — see `just/container.just`.
3. Roll the production deployment to the new image, e.g.
   `just cluster-rollback-live-refine <image_tag>`.
4. Confirm with `just cluster-status` / `just cluster-logs-live-refine`.

## Compute isolation: the `production` k8s namespace

Data-plane versioning (above) is separate from compute isolation. All current
`infra/k8s` components — `pruner`, `live-refine`, `skeet-publish`, the three
exporter cronjobs, `optimise`, and the `OnePasswordItem` secrets — live in the
**`production`** namespace (`infra/k8s/namespace.yaml`, applied before the
secrets in `just cluster-1password-secrets-install`). `NAMESPACE := "production"`
(defined in `just/cluster.just`) is the target of every deploy / rollout / logs /
scale / status recipe (`just/cluster-deploy.just`) and of the `ghcr-pull-secret`
creation (`just/cluster.just`). The feed runs on Fly, not k8s.

A worktree that needs to run a *changed* component runs it in its own namespace,
sharing the backend stores; unchanged components are not duplicated. Keep it
minimal — don't replicate stores or add per-worktree infra beyond the changed
component.

OTel `deployment.environment` stays `hetzner`: it names *where* the code runs
(the cluster), not the prod/staging role. The namespace alone carries the split.

## Build & deploy

Build and deployment stay local on the laptop; all invocations are captured in
the Justfile (`just/container.just` build/push, `just/cluster.just` provisioning,
`just/cluster-deploy.just` deploy/rollback, `just/refine.just` promotion).
