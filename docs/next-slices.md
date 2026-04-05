# Next Slices

## Slice 12: Optimisations of pruning, refining, and feeding

### Smells to be investigated / addressed

1. looking at cpu and network usage in hetzner:
    * we can see it going up and down:
        * cpu goes from 100% to 200% over a period of about 30 secs
        * network bandwidth goes from 0 to 80Mbps over similar time-scale
    * this feels a bit like a bottleneck being encountered periodically like a buffer being filled and meeting a limit
    * we also log every 30 seconds, so it's possible this is somehow blocking things (as it sits at end of pipeline)
2. looking at traces for operations like `get_by_id` these take about 10 seconds and make multiple `read_fragment` calls in `lance::io::exec::filtered_read`
    * this kinda looks like a table scan; couldn't this be an index lookup instead?
    * also, could we add a dump of the plan to the trace (in `plan_scan`) so we can see what it is doing?
    * I see similar traces for things like `exists`
3. the read of the feed itself takes several seconds
    * this could be caused by the same or similar problems as 2 i.e. long scans

### Optimisation ideas

* live-refine: when we find a new set of images to score:
    * we can dispatch multiple calls in parallel to openai as we're largely waiting on them to respond (it's i/o bound)
    * once we have some scores, we can batch-save them to lancedb (lancedb recommends batch-saving to reduce fragmentation)

### Benchmarking

* we should be able to measure the maximum possible speed the pruner can run by taking the jetstream stage only and running that on it's own
    * we should be able to run a minimal cluster which just runs this step

### Tasks

* [ ] for each of the smells break out possible causes and options for fixes. this may also include adding more visibility through opentelemetry or other tracing
* [ ] similarly, for each of the ideas
* [ ] similarly, for each of the benchmarks
* [ ] ...

## Slice 13: Add /admin area in skeet-feed that allows blocking of skeets

### Target

A home page (`/`) to show what is currently visible in the feed i.e. what you would see on the feed right now.

An `/admin` area where we show what is currently appearing in the feed (as above) + any that have been blocked. Each of the items should be able to be blocked, which makes them disappear from the feed.

Protect the `/admin` area behind GitHub OAuth login. Users authenticate via GitHub; their username is checked against an allowlist stored in a fly.io secret. No credentials are stored in the app — only an ephemeral session records that the user has the admin role.

### Tasks

#### Home and Admin UI with blocking capability
* [ ] produce a view which shows same data as in feed, with an additional block button and display of whether it is blocked, on `/admin`
    * we should try to share if possible between views
* [ ] a new `blocked_skeet_v1` table which holds which skeets have been marked as blocked. This is mostly expected to be empty.
    * the key of the table is the at URI and the table should have a `blocked` boolean column
    * this should be accessible via `SkeetStore`
* [ ] needs a suite of both integ and unit tests which prove that skeets can be filtered out if blocked and also can be unblocked

#### GitHub OAuth App setup
- [ ] Register a new OAuth App at GitHub → Settings → Developer settings → OAuth Apps; set the authorization callback URL to `https://<app-name>.fly.dev/auth/callback`
- [ ] Store the client ID and secret as fly.io secrets: `fly secrets set GITHUB_CLIENT_ID=… GITHUB_CLIENT_SECRET=…`
- [ ] Set the admin allowlist as a fly.io secret: `fly secrets set ADMIN_USERS=mikemoraned` (comma-separated GitHub usernames)
- [ ] Generate a random session signing key and store it: `fly secrets set SESSION_SECRET=$(openssl rand -hex 32)`

#### Dependencies
- [ ] Add `oauth2 = "5"` and `reqwest = { version = "0.12", features = ["json"] }` to the workspace `Cargo.toml`; these handle the OAuth2 flow and the GitHub API call to resolve the authenticated username
- [ ] Confirm `tower-sessions` is already available via cot; if a session store beyond the default is needed (e.g. for multi-instance), add `tower-sessions-redis-store` or equivalent

#### Auth routes (new `AuthApp`)
- [ ] Create `src/auth_app.rs` implementing `cot::App` with three routes: `GET /auth/login`, `GET /auth/callback`, `GET /auth/logout`
- [ ] `/auth/login`: build an OAuth2 authorize URL (scope `read:user`), store the CSRF state token in the session, and redirect the user to GitHub
- [ ] `/auth/callback`: verify the CSRF state, exchange the authorization code for an access token, call the GitHub `GET /user` API to retrieve the username, check the username against `ADMIN_USERS`, and — if matched — set `role=admin` in the session and redirect to the URL stored in `return_to` (or `/admin` by default)
- [ ] `/auth/logout`: clear the session and redirect to `/`
- [ ] Register `AuthApp` in the `Project::register_apps` with prefix `/auth`

#### Admin guard middleware
- [ ] Write an axum `middleware::from_fn` called `require_admin` that reads `role` from the session; if not `"admin"`, stash the current request URI in `return_to` and redirect to `/auth/login`
- [ ] Apply this middleware as a `route_layer` on the `/admin` sub-router so it runs only for admin routes, not for public routes or `/auth/*`
- [ ] Verify that cot's `SessionMiddleware` is ordered before the admin guard in the middleware stack so the session is available

#### Local development
- [ ] Create a second GitHub OAuth App (or reuse with an additional callback URL) pointing at `http://localhost:8080/auth/callback` for local testing
- [ ] Document the required environment variables in a `.env.example` file: `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `ADMIN_USERS`, `SESSION_SECRET`
- [ ] Add a note in the README on how to run locally with these env vars (e.g. via `cargo run` with `dotenv` or `export` commands)

#### Verification (by unit and integ tests)
- [ ] Confirm that unauthenticated `GET /admin` redirects to GitHub login
- [ ] Confirm that after GitHub login with an allowlisted username, the user lands on `/admin` with full access
- [ ] Confirm that after GitHub login with a non-allowlisted username, the user sees a 403 or a "not authorized" message (not a silent redirect loop)
- [ ] Confirm that `GET /auth/logout` clears the session and subsequent `/admin` requests redirect to login again
- [ ] Confirm that the CSRF state parameter is validated on callback and a tampered `state` is rejected

