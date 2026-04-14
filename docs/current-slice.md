# Current Slice: Slice 13 — Add /admin area in skeet-feed for manual quality appraisal of skeets and images

### Target

A home page (`/`) to show what is currently visible in the feed i.e. what you would see on the feed right now.

An `/admin` area where we show what is currently appearing in the feed (as above) + any that have been blocked and band they are in based on automatic and manual appraisal. Each of the items should be able to be manually appraised into the bands below. By default, without manual action, all images are unappraised.

There are four bands in order of worst -> best quality:
* Low Quality:
  * these don't match the general layout at all and should all be blocked earlier in the Prune stage
  * scores: 0.0 -> 0.25
* Medium Low Quality:
  * these technically match the general layout we want but don't match the theme. Ideally we'd also block these at Prun 
  * scores: 0.25 -> 0.5
* Medium High Quality:
  * these match general layout we want, and match the theme, but they are not great
  * scores: 0.5 -> 0.75
* High Quality:
  * matches general layout, and also are great exemplars of original idea or are really interesting even if they don't match the original goal
  * scores: 0.75 -> 1.0

Note that we need to separate appraisal of the *skeet* from the image. The /admin area needs ability to do both i.e. appraise skeets and images into Bands. The default view for the admin area should be the skeet appraisal.

Anything image or associated skeet in the Low or Medium Low Quality bands should cause the associated skeet to not appear in the feed i.e. be filtered out. When sorting by quality, sort best to worst. A manual appraisal always supercedes an automatic appraisal.

Protect the `/admin` area behind GitHub OAuth login. Users authenticate via GitHub; their username is checked against an allowlist stored in a fly.io secret. No credentials are stored in the app — only an ephemeral session records that the user has the admin role.

### Tasks — Preparatory Refactors

#### Domain types (`shared` crate)
- [x] Add a `Band` enum: `Low`, `MediumLow`, `MediumHigh`, `HighQuality`. Implement `Ord`, `Display`, `FromStr`.
- [x] `Band::from_score(Score)` using half-open intervals: `[0.0, 0.25)` Low, `[0.25, 0.5)` MediumLow, `[0.5, 0.75)` MediumHigh, `[0.75, 1.0]` HighQuality.
- [x] `Band::is_visible_in_feed(self)` — true for `MediumHigh` and `HighQuality` only.
- [x] Unit tests for boundary cases (0.0, 0.25, 0.5, 0.75, 1.0).
- [x] Add an `Appraiser` enum capturing identity + provider of whoever made an appraisal. Initial single variant: `GitHub { username: String }`. Designed so future providers (e.g. Bluesky) can be added as new variants without breaking existing data.
- [x] Implement `Display` / `FromStr` for `Appraiser` using a `provider:identifier` wire format (e.g. `github:mikemoraned`) — single string column in storage, forward-compatible with new providers.
- [x] Unit tests for `Appraiser` parse/display roundtrip and rejection of malformed strings.

#### `skeet-web-shared` crate (new)
- [x] Create a new workspace member `skeet-web-shared` for parts that skeet-inspect and skeet-feed will share.
- [x] Move the `Store`/`StoreLayer` middleware out of skeet-inspect into skeet-web-shared.
- [x] Move shared view types and helpers (`FeedEntry`, `to_feed_entry`) into skeet-web-shared.
- [x] Vendor htmx (single `htmx.min.js`) as a static asset, served via cot's static-files support.
- [x] Add a base layout template (loads htmx) usable from both crates.
- [x] Update skeet-inspect to depend on skeet-web-shared and use the moved code; verify `just inspect` still works.

#### Storage: cursor-paged listing (`skeet-store`)
- [x] Add `SkeetStore::list_summaries_page(before: Option<DiscoveredAt>, limit: usize) -> (Vec<StoredImageSummary>, next_cursor)` — cursor-based paging by `discovered_at` desc.
- [x] Unit tests: first page; subsequent pages; end-of-data; concurrent insert during paging.

#### HTML infrastructure in `skeet-feed`
- [x] Add cot template support to skeet-feed (`#[derive(Template)]`, `templates/` directory).
- [x] Depend on `skeet-web-shared` for store middleware, vendored htmx static files, and shared view types.

### Tasks — Implementing Appraisal

#### Storage: manual appraisal tables (`skeet-store`)
- [x] Add `manual_skeet_appraisal_v1` table: `skeet_id` (string, AT URI, key), `band` (string), `appraiser` (string, `Appraiser` wire format), `appraised_at` (timestamp). Presence of a row = manual appraisal exists; delete to revert to automatic.
- [x] Add `manual_image_appraisal_v1` table: `image_id` (string, key), `band`, `appraiser`, `appraised_at`.
- [x] `SkeetStore` methods: `set_skeet_band(&SkeetId, Band, &Appraiser)`, `clear_skeet_band`, `get_skeet_band`, `list_all_skeet_appraisals` — and the four image equivalents. `get`/`list` return the stored `Appraiser` alongside the band so the admin view can show who made each call.
- [x] Unit tests for set/get/clear/list round-trips on each table, including appraiser preservation.

#### Effective band logic
- [x] Define a function (in `shared` or `skeet-web-shared`) that, given an image score + manual image band + manual skeet band + sibling-image bands, computes:
  - per-image effective band: `manual_image.unwrap_or(Band::from_score(score))`
  - per-skeet auto band: worst per-image effective band across the skeet's images
  - per-skeet effective band: `manual_skeet.unwrap_or(auto_skeet)`
  - skeet visible in feed: `effective_skeet_band.is_visible_in_feed() && every image effective band is visible`
- [x] Unit tests: no manual; manual demote skeet; manual promote skeet; one bad image taints the whole skeet; manual skeet override beats per-image overrides.

#### Feed filter integration (`skeet-feed`)
- [x] Update `FeedCache::refresh()` to also load manual skeet + image appraisals (full-table scans — both tables are tiny).
- [x] Update `get_feed_skeleton` to use the effective-band visibility rule instead of `score >= config.min_score`.
- [x] Remove the `min_score` field from `FeedConfig` (and the corresponding CLI flag) — band thresholds replace it. Update `fly.staging.toml` and the Justfile feed targets accordingly.
- [x] Integ tests: skeet visible by default; manually demoting the skeet hides it; manually demoting one of its images hides it; promoting only the skeet is not enough when its image is Low (lowest band across skeet + images determines visibility); promoting both skeet and image shows it again.

#### Cache-control for feed endpoints
- [x] When a request to `get_feed_skeleton` or `home` includes `Cache-Control: no-cache`, force a `FeedCache::refresh()` instead of serving potentially stale cached data.
- [x] Return a `Date` response header set to the wall-clock time when the cache was last refreshed (not the current request time), so clients can see how fresh the data is.
- [x] Use `Cache-Control: no-cache` in integ tests after appraisal mutations to guarantee the feed reflects the latest store state.

#### Home view (`/`)
- [x] New handler `home` rendering the currently-visible feed items as HTML.
- [x] Sort: best-to-worst by score (existing feed cache ordering).
- [x] Per item: thumbnail (annotated image), score, AT URI, link to bsky.app. No admin controls. No paging — bounded by feed size.

#### Add default local appraiser (local admin)
- [x] create new `Appraiser::LocalAdmin` enum value
- [x] when feed is run locally with a `--local-admin` flag then `Appraiser::LocalAdmin` is the default appraiser used. This flag is disabled by default
  * the intent is that this local-admin is used only when running from local desktop

#### Admin view (`/admin`)
- [x] New handler `admin` rendering all stored items, sorted by `discovered_at` desc.
- [x] Two sub-views: skeet appraisal (default) and image appraisal.
- [x] Cursor-based paging using `list_summaries_page`, 10 items at a time.
- [x] htmx "load more": initial render shows the first 10 items + a sentinel `<div hx-get="/admin?cursor=..." hx-trigger="revealed" hx-swap="outerHTML">` that fetches the next 10 when scrolled into view. Server returns HTML fragments.
- [x] Per item: thumbnail, score, automatic band, manual band (if any), effective band, band selector (4 buttons + "clear manual").
- [x] htmx band-update: each band button does `hx-post="/admin/appraise/skeet/{id}"` (or `image/{id}`) and swaps the row in place via `hx-swap="outerHTML"`. The handler reads the current `Appraiser` from the session and passes it to the `SkeetStore` set method.
  * this should fail, with an internal error, if no Appraiser has been set
  * an Appraiser of `Appraiser::LocalAdmin` may be set when the server is started with `--local-admin` (see above)
- [x] Integ tests: paging returns expected items in expected order; setting a manual band updates the row and the underlying table; clearing reverts to automatic.

#### Operational — do first (manual, not code)
- [ ] Register a GitHub OAuth App for staging: GitHub → Settings → Developer settings → OAuth Apps → New OAuth App. Callback URL: `https://bobby-staging.fly.dev/auth/callback`. Store in 1Password as `bobby-github-oauth-staging-client-id` and `bobby-github-oauth-staging-client-secret` (vault: Dev, category: password, value in `password` field).
- [ ] Register a second OAuth App for local dev. Callback URL: `http://localhost:8080/auth/callback`. Store in 1Password as `bobby-github-oauth-local-client-id` and `bobby-github-oauth-local-client-secret` (vault: Dev, category: password, value in `password` field).
- [ ] Generate a session secret and store in 1Password: `just generate-session-secret`.
- [ ] Split `bobby.env` into two env files, each containing the shared secrets (R2, OpenAI, OTEL) plus environment-specific OAuth vars:
  - `bobby-local.env` — shared secrets + local OAuth app's `BOBBY_GITHUB_CLIENT_ID`, `BOBBY_GITHUB_CLIENT_SECRET`, `BOBBY_SESSION_SECRET`, `BOBBY_ADMIN_USERS=mikemoraned` (all as `op://` references).
  - `bobby-staging.env` — shared secrets + staging OAuth app's `BOBBY_GITHUB_CLIENT_ID`, `BOBBY_GITHUB_CLIENT_SECRET`, `BOBBY_SESSION_SECRET`, `BOBBY_ADMIN_USERS=mikemoraned` (all as `op://` references).
  - Delete `bobby.env` after the split.
- [ ] Update all `op run --env-file bobby.env` references in `just/*.just` to use `bobby-local.env` (all are local-dev commands). Update `deploy_staging_secrets` to use `bobby-staging.env`.
- [ ] Set Fly secrets via `deploy_staging_secrets` (which now reads from `bobby-staging.env`).

#### Auth: cot session bootstrap (depends on Operational)
- [ ] Wire `cot::middleware::SessionMiddleware` into `FeedProject::middlewares()`. Default in-memory store is fine (single-instance Fly machine; re-login after suspend is acceptable).
- [ ] Load session signing key from `BOBBY_SESSION_SECRET` env var.
- [ ] Load admin allowlist from `BOBBY_ADMIN_USERS` (comma-separated GitHub usernames).
- [ ] Load GitHub OAuth client id/secret from `BOBBY_GITHUB_CLIENT_ID` / `BOBBY_GITHUB_CLIENT_SECRET`.

#### Auth: GitHub OAuth routes
- [ ] Add `oauth2 = "5"` to workspace `[dependencies]`.
- [ ] New module implementing routes `GET /auth/login`, `GET /auth/callback`, `GET /auth/logout`, registered under `/auth`.
- [ ] `/auth/login`: build an OAuth2 authorize URL with scope `read:user`; store CSRF state in cot session; redirect to GitHub.
- [ ] `/auth/callback`: verify CSRF state; exchange code for access token; call GitHub `GET /user`; check username against allowlist; on success, set `role=admin` and store `Appraiser::GitHub { username }` in the session, then redirect to `return_to` or `/admin`; on failure, return 403 with a clear message (no silent redirect loop).
- [ ] `/auth/logout`: clear the session; redirect to `/`.

#### Admin guard
- [ ] Implement a middleware (built on cot's session primitives) that checks for `role=admin` in the session; if absent, store the current request URI in `return_to` and redirect to `/auth/login`.
- [ ] Apply only to `/admin/*` routes; ensure `SessionMiddleware` is ordered before it.

#### Verification (unit + integ tests)
- [ ] OAuth tests use a mocked GitHub `/user` response (no real round-trips).
- [ ] Unauthenticated `GET /admin` redirects to `/auth/login`.
- [ ] Allowlisted user lands on `/admin` after login.
- [ ] Non-allowlisted user gets 403, not a silent redirect loop.
- [ ] `/auth/logout` clears the session; subsequent `/admin` requests redirect to login again.
- [ ] Tampered CSRF state on callback is rejected.
