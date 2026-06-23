# Current Slice: robust, low-impact firehose consumption + `skeet-prune` review/re-org

### Targets

#### Focus on longer-term maintainance

Refactor, review and minimisation of code for longer-term maintenance so "I can walk away from this for a while".

* the general expectation is that I want to be able to leave this repo for a while and go work on other stuff, and not need to worry about surprising code or lingering cruft/weirdness.
* split out code into sub-dirs based on role e.g. crates are at top-level in repo, and so should go into a subdir; follow generally accepted conventions where possible.

The general bias is to refactor towards patterns and structures that are the best practice for what kinds of things the `skeet-prune` crate is doing.

#### Firehose robustness, specifically

`skeet-prune` already consumes the firehose the efficient way — Jetstream (not the raw `com.atproto.sync.subscribeRepos` CBOR firehose) via `jetstream-oxide`, with `JetstreamCompression::Zstd`, a server-side `wantedCollections=app.bsky.feed.post` filter, and a shuffled four-endpoint list for connect-time failover. The transport is already lean; the gap is *consumption robustness*, not mechanism. Two failure-mode defects remain, both in `firehose.rs` / `firehose_stage.rs`:

- **Reconnects silently lose data.** `JetstreamConfig.cursor` is never set (we build the config with `..Default::default()`), so every reconnection starts in live-tail. A silent connection takes up to `recv_timeout` (30s) to notice, then up to ~5s × endpoints to re-establish — and every post in that whole window is gone, unobserved. On a redeploy or a flaky network this quietly punches holes in the stream we never see.
- **The reconnect loop has no backoff.** On connect failure `firehose_stage::run` does `warn; continue` with zero delay, and `max_retries: 0` disables the library's own retry — so during a real outage the DIY loop retry-storms all four public Jetstream instances as fast as it can. That's exactly the "unnecessary load on the central servers" we don't want to be a source of.

Goal: survive disconnects without losing observed posts, and stay a polite client even when things are on fire — without touching the (already-good) transport. The pipeline is already idempotent — the store is keyed by content hash / `SkeetId` — so replaying a few seconds of duplicate events on reconnect is safe and collapses to the same rows. That idempotency is the precondition that makes cursor-based resumption viable here.

Apply the cursor change first, then the backoff: backoff deliberately lengthens the disconnect gap, so the cursor (which makes gaps lossless) needs to be in place *before* we make gaps longer on purpose.

### Tasks

#### Group 0: longer-term maintainance changes

Each crate related to `skeet-prune` gets at least one full human pass (ideally): read all code, delete dead code, rework anything surprising, enforce the house rules along the way — `lib.rs` under 300 lines (extract modules if over), no `#[allow(dead_code)]`, no `unwrap`/`expect` in non-test code without a justified allow, and strip comment-rot (slice/phase/PR/task refs — see [[no-slice-phase-refs-in-code-comments]]). Note any non-obvious findings per crate. (Crate-specific NewType/error/visibility cleanups for `bluesky`/`shared` are listed in the *remaining-crates* 1.0 slice's Shared/support area; apply them wherever that crate's pass happens first.)

Tasks:
* ...

#### Group 1: bobby.houseofmoran.io should fall back to older data when newer unavailable

It's possible there could be an outage of the backing pruner process for > 48h which means what is shown on the feed or on website could expire gradually. I think what I'd like to do here is add a fallback mechanism where, for preferred Order + Limit, it will try successively older lists if it doesn't find anything in the preferred one. So, here's how it would roughly work:
* Finds all feeds available and orders them by age limit (i.e. 48h before 7d before 1y)
* When looking for a preferred feed (e.g. `quality-48h`) if that feed is empty, then it will find the next oldest available feed with the same ordering, in this case `quality-7d`. If that's also empty or missing then it should go to next.

This way when there is some sort of outage it gracefully degrades to older data.

**Decisions**
* **Home for the fallback: `skeet-publish`.** It already owns `FeedSource`/`PublishedImagesSource`/`RedisFeedSource`, the `PublishedList`/`PublishedListCatalog` keys, and `connect` — the fallback is one more source built from those. `skeet-feed` just wires it; `skeet-appraise` keeps its own `AvailableFeeds` (its dropdown/`resolve`/`UnknownFeed` policy differs and isn't a fallback).
* **Discovery is per-request, via the catalog** (the same shape `skeet-appraise` already uses). Re-reading the catalog + re-checking emptiness on each call is what makes degradation *live*: an outage that empties the preferred list mid-run, or a newly-published older list, is picked up without a restart. Cost is one extra `SMEMBERS` per request on the happy path.
* **Fallback chain = same `Order`, window ≥ preferred, ascending by `Limit::window()`.** "Next oldest with the same ordering" (doc) ⇒ stay within the preferred `Order` (e.g. `quality-48h` → `quality-7d` → `quality-4w` → `quality-1y`), never fall back to a *newer* window, never cross `Order` (quality→recency is out of scope). A missing preferred just means the chain starts at the next-oldest available list. "Empty or missing" is a single trigger because `PublishedList::read` already returns `[]` for both.

Tasks:
* [x] **Candidate-chain selection in `skeet-publish` (pure, tested first).** Add the logic that turns *(catalog specs + preferred `(Order, Limit)`)* into the ordered fallback chain: keep specs whose `Order` equals the preferred and whose `Limit::window()` ≥ the preferred's, sort ascending by window, yielding `[preferred-or-next-oldest, …, oldest]`. Keep it a pure function over `&[(Order, Limit)]` so it unit-tests without redis (examples + a proptest invariant: output is same-`Order`, non-decreasing window, and contains nothing newer than preferred). TDD: stub returning `vec![]`, write the ordering assertions (failing), then implement. (`Limit::window()` already exists; reuse it rather than re-deriving age.)
* [x] **`FallbackFeedSource` implementing both `FeedSource` and `PublishedImagesSource`.** New type in `skeet-publish` holding the publish redis url + the preferred `(Order, Limit)`. Per call: discover the catalog (`PublishedListCatalog::read` → `PublishedList::spec`), build the candidate chain (task above), then read each candidate (reusing `RedisFeedSource`) in order and return the **first non-empty** result — `FeedSkeleton` for `skeleton()`, `PublishedImages` for `published_images()` — carrying that list's `refreshed_at`. If every candidate is empty/missing, return the last (empty) result so the surface still renders. `examined_count()` is list-independent (single key) — delegate to the existing read unchanged. Unit-test first-non-empty selection against a fake reader (stub: always returns the preferred), then implement. Note: the happy path is one catalog read + one list read; only an outage walks further. (Decide whether to reuse `RedisFeedSource`'s transient-retry per candidate or read through one shared connection — default to reusing `RedisFeedSource` as-is for simplicity unless the per-candidate reconnect proves costly.)
* [x] **Wire `skeet-feed` onto the fallback sources.** In `skeet_feed.rs`, replace the two fixed `RedisFeedSource` constructions with `FallbackFeedSource`: Bluesky skeleton preferred `quality-48h`, website grid preferred `quality-4w`, both against `redis_publish_url`. Update the two explanatory comments (`skeet_feed.rs:77`/`:84`) to describe the preferred-with-fallback behaviour, not a fixed list. Behaviour must be byte-identical when the preferred list is populated. Verify `just clippy` + `just test-no-docker`.
* [x] **Integration test the degradation through the HTTP surface.** Using the existing redis-backed test infra (testcontainers, as `skeet-publish`/`skeet-appraise` tests do), assert via `skeet-feed`'s public HTTP endpoints: (a) with `quality-48h` empty/absent but `quality-7d` populated, `getFeedSkeleton` and the homepage serve the `quality-7d` data; (b) with `quality-48h` populated it wins; (c) the `examined_count` banner still renders regardless. Runs under `just test` (Docker) — Claude verifies the non-Docker parts with `just test-no-docker`; the container test is a human/CI step. Run `just mutants-on-diff` once the chain + source land.
  * Done in `crates/skeet-feed/tests/feed_integration.rs` (the subprocess-against-testcontainers infra). Caveat: the homepage grid's preferred is `quality-4w`, so its fallback chain can only widen (4w → 1y), never down to the narrower `quality-7d`. The test therefore exercises the homepage degrading `quality-4w` → `quality-1y` (and the feed `quality-48h` → `quality-7d`), which matches the actual wiring rather than the literal "homepage serves quality-7d" wording. mutants-on-diff: the pure chain + selection-walk mutants are caught by the unit tests; the redis-I/O delegations (`available_specs`/`examined_count`) are only caught under Docker (CI).

#### Group 2: cursor-based resumption

- Track `info.time_us` of the last processed event; set `JetstreamConfig.cursor` (a `DateTime<Utc>`) on reconnect, rewound ~5s for gapless playback. Currently `None` ⇒ every reconnect live-tails and drops the gap.
- Safe to replay because the store is idempotent (keyed by content hash / `SkeetId`).
- In-memory across reconnects is the 80%; persisting the cursor (closes the restart gap) is optional.

Tasks:
* ...

#### Group 3: reconnect backoff

- Add exponential backoff with jitter between reconnect *cycles* (one cycle = one pass over all shuffled endpoints), capped ~30–60s; reset after a connection stays up.
- Currently `warn; continue` with no delay, and `max_retries: 0` disables the library's retry — so an outage retry-storms all four instances.
- Keep the DIY loop (it gives endpoint rotation, which `max_retries` doesn't); just add the delay.

Tasks:
* ...
