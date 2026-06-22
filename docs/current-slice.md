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

Tasks:
* ...

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
