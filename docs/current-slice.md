# Current Slice: 1.0 public "Bobby" feed

### Target

`bobby.houseofmoran.io`: same underlying code as the staging version (`bobby-staging.houseofmoran.io`), but promoted to a real 1.0 public feed:

* published on Bluesky as a feed called "Bobby" (i.e. not "Bobby Dev")
    * has a small inline blurb explaining what this is, which is shared with the website (banner below)
* tracking of usage via plausible.io
* a small banner at top which shows:
    * an explanation of what this is (the shared blurb)
    * a small qr code for the `https://bobby.houseofmoran.io/` url
    * instructions on how to subscribe to the feed on bluesky (with a link to it)
    * summary data of how many images examined (should be precalculated by the publisher and saved in redis)

Also promote the appraisals site to its own production URL — `bobby-appraisals.houseofmoran.io`:

* nothing additional needed in code, i.e. it's the same thing, just running under an additional url
* however, a new github app will need created for auth purposes

### Decisions (confirmed 2026-06-14)

* **Deploy topology: new, separate Fly apps.** Production = a new `bobby` app (`fly.production.toml`) and a new `bobby-appraisals` app (`fly.appraise.toml`), each running the *same* GHCR image as its staging counterpart, just under the production hostname and config. Staging's `fly.staging.toml` / `fly.appraise-staging.toml` stay untouched. This mirrors the prod/staging separation already done for k8s — prod and staging are distinct compute that share the backend stores.
* **"Images examined" = estimated images *processed* by the pruner.** We want to show how many images were seen/processed, not just how many were saved. The pruner doesn't persist a seen-count, so the publisher *estimates* it: count the distinct images that made it through refine scoring (distinct images with a known-version score), then scale up by the inverse of a hard-coded save rate (~0.2%, i.e. ×500) to estimate how many were originally seen. The publisher precalculates this estimate and writes it to redis; the feed reads it for the banner unchanged. No new pruner-side counter. The save rate is hard-coded in the publisher (`SAVE_RATE_PERCENT`); revisit it if the real save percentage drifts.
* **Plausible: feed website only.** Only `bobby.houseofmoran.io` gets the plausible script. The appraisals site is auth-gated/internal — no tracking there.
* **QR code: server-side rendered inline SVG.** Generate the QR for `https://bobby.houseofmoran.io/` with a Rust crate (e.g. `qrcode`) and inline it as SVG in the banner. No external calls, no committed binary asset.

### Tasks

#### Feed website: shared blurb + banner

* [ ] **Single source of truth for the blurb.** Define the shared explanatory blurb once (a `const`/function in a shared place the feed can render and the registration bin can read) so the Bluesky feed `description` and the website banner can't drift. Keep the existing tagline or fold it into the blurb — one canonical wording.
* [ ] **Render the banner at the top of `home.html`.** Add a banner above the grid showing: the shared blurb; an inline server-rendered QR SVG for `https://bobby.houseofmoran.io/`; subscribe-to-the-feed instructions with a link to the feed on Bluesky (derive the link from `FeedParams::feed_uri()` / a `bsky.app` URL); and the "images examined" summary count. Keep it small and unobtrusive; style inline like the rest of `home.html`.
* [ ] **Server-side QR generation.** Add the `qrcode` crate (stable, non-`-pre`); render the production URL to inline SVG. Pure function, unit-tested for non-empty/well-formed output. The encoded URL comes from config (the feed hostname), not hardcoded.

#### "Images examined" stat (publisher → redis → feed)

* [x] **Publisher precalculates the count and writes it to redis.** In `skeet-publish`, compute the total scored/appraised image count during the publish cycle (it already loads the scored data) and write it under a versioned redis key following the existing `<SCHEMA_VERSION>-<type>` convention (e.g. `v3-examined-count`) — derive the prefix from `SCHEMA_VERSION`, don't hardcode. Reuse the existing redis client/serialisation path.
    * Note: the saved count comes from a new `SkeetStore::count_scored_images(known_versions)` (distinct images with a known-version score, fresh table scan — not the scores cache, which can lag). The publisher scales it by the inverse of `SAVE_RATE_PERCENT` (0.2%) to write an *estimated processed* count, not the raw saved count (see the Decisions note).
* [x] **Feed reads the count for the banner.** Extend the published-images source (or add a sibling read) so the home handler can fetch the count and pass it to the template. Tolerate the key being absent (feed renders without the number rather than erroring) — covariant read.
    * Note: `home.html` already renders the count inline (a minimal `<p class="examined">` line) so the template field isn't dead — the banner task folds this into the full banner.

#### Plausible tracking (feed only)

* [ ] **Add the plausible.io script to `home.html`** with `data-domain="bobby.houseofmoran.io"`. Confirm it only loads on the production host (so staging/local don't pollute stats) — gate via config rather than hardcoding the domain into the template.

#### Bluesky: register the real "Bobby" feed

* [ ] **Register the production feed as "Bobby".** Run `register-feed` for the production hostname with `--feed-name bobby` / `--display-name Bobby` and the shared blurb as `--description`. Add a `register-feed-production` just recipe pointing at the production hostname (don't change the staging `bobby-dev` defaults). The `bobby-dev` staging feed stays as-is.

#### Production deploy plumbing (new Fly apps)

* [ ] **Production feed Fly app.** Add `fly.production.toml` (app `bobby`, production hostname, same GHCR `skeet-feed` image) and the just recipes to deploy it (secrets import from a new `bobby-feed.env`, app deploy, end-to-end check) — mirror the `deploy_feed_staging*` recipes. Wire `bobby.houseofmoran.io` DNS / `did:web` to the new app.
* [ ] **Production appraisals Fly app + new GitHub OAuth app.** Add `fly.appraise.toml` (app `bobby-appraisals`, `bobby-appraisals.houseofmoran.io`, same `skeet-appraise` image). Create a new GitHub OAuth app with the production callback URL (`https://bobby-appraisals.houseofmoran.io/auth/callback`). Add the deploy just recipe mirroring the staging appraise recipe. New `bobby-appraisals.env` reuses the shared backend-store / OTel / redis items from `bobby-appraisals-staging.env` unchanged (R2, SSE-C, redis URLs are shared stores), and points at **new** 1Password items for the production-only secrets:
    * `BOBBY_GITHUB_CLIENT_ID=op://Dev/bobby-github-oauth-appraisals-production-client-id/password`
    * `BOBBY_GITHUB_CLIENT_SECRET=op://Dev/bobby-github-oauth-appraisals-production-client-secret/password` (mirrors the existing `...-appraisals-staging-...` pair, `staging`→`production`)
    * `BOBBY_SESSION_SECRET=op://Dev/bobby-session-secret-appraisals-production/password` — a **dedicated** production session secret, not the shared `bobby-session-secret` staging uses, so prod/staging sessions can't cross over.
  * **1Password items to create (in the `Dev` vault):** `bobby-github-oauth-appraisals-production-client-id`, `bobby-github-oauth-appraisals-production-client-secret` (from the new GitHub OAuth app), and `bobby-session-secret-appraisals-production` (freshly generated random secret). All other `op://Dev/...` refs are reused as-is.

#### Wrap-up

* [ ] **Capture all new invocations in the Justfile** (register-production, both production deploys) and run `just clippy` + `just test-no-docker`.
