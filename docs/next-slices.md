# Next Slices

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

## Slice 14: reducing unintentional bias

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