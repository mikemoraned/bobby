# Current Slice: Make statistics more visible / understandable

### Target

As of 27th June we say "(22,223,000 images checked so far)" on https://bobby.houseofmoran.io but this doesn't make it clear how few of these actually match the archetype.

We'd like to change this to say something like "(400,000 images checked over past 2 days, of which 46 (0.01%) match what we are looking for)". This should show human-readable numbers and days e.g. time rounded to nearest hour/day/week/month/year multiple, and percentages shown to a round two decimal places.

I'd also like to give a better experience when they get to the end of the page e.g. show something like "You've reached the end of the images found so far! Next one expected to be found in X hours" and then make it so that the page automatically reloads. Perhaps add a small JS countdown timer.

### Tasks

We'll get there in gradual steps:
* [x] do a supporting refactor which factors out a `content_statistics_stage` stage which sits after the current `save_stage`. It's only job is to receive the `ContentCounts` from previous stages. So effectively we split `save_stage` into a stage which just saves to the store, and a `content_statistics_stage` which does everything else that stage currently does. (Consequence: `saved` is folded sink-side in `save_stage` today; once `Status` moves downstream it must ride the data plane save→stats — superseding the firehose-slice "keep saved sink-side" note.)
* within `skeet-store`:
    * [x] Record prune statistics:
        * [x] create new `Statistics` trait (impl'd by SkeetStore) which can store prune statistics i.e. something similar to what we are currently saving in otel metrics:
            * Count of Skeets seen on firehose
            * Count of Images examined i.e. how many were looked at even before they were saved
            * Count of Images saved as candidates
            * These are counts within a particular interval (see below), which should also be recorded with a start and end timestamp
        * [x] backfill statistics (via one-off cli which we'll delete afterwards):
            * because we've not been running this statistics gathering already, we don't have all the information we need. In particular, we don't have a count of skeets that were seen but not saved.
            * so, we should write a cli which is resumable from where it got to via:
                1. Using the `Images` port, find oldest image saved, and extract the date
                2. Using the `Statistics` port, find interval for which we have some already statistics (initially, this should be empty i.e. None)
                3. Find the max of oldest Image date and newest end of interval, with the intent of finding the intervals we still have to backfill. 
                4. Working forward from this starting date in steps of 1 hour, find all Images saved during that interval:
                    * this becomes `images_saved` of PruneStats
                    * we calculate backwards to `skeets_seen` and `images_examined` using `SAVE_RATE_PERCENT`, with the assumption that `skeets_seen == images_examined`
                    * in other words `skeets_seen = images_examined = (images_saved/SAVE_RATE_PERCENT)` as `images_saved = images_examined * SAVE_RATE_PERCENT`
                    * create and `record` a `PruneStats` instance based on these numbers and this interval
            * any changes we need to make to `Images` and `Statistics` ports to support this should be kept even after we delete the cli
        * [x] Update pruner, in new `content_statistics_stage` so that it saves these stats to `Statistics` every time it updates the logged output. It should save a new record of stats for each interval e.g. from timestamp T1 to T2, 20 skeets seen, etc. (These numbers already exist once per interval as `ContentCounts` in `Status::log_summary` — `posts`/`images`/`saved` map 1:1. Still needed: wall-clock `DateTime<Utc>` interval bounds, since the cadence is monotonic `Instant`; and a store-owned record for `Statistics::record` populated from `ContentCounts`, as `skeet-store` can't import the pruner's type.)
    * [x] Add ability of `Statistics` trait to calculate:
        * a sum of prune counts seen over a particular interval (based on saved prune records above), which is the number of images examined — done as `interval_counts`, which returns the summed `PruneStats` (examined plus skeets-seen/saved) for the window rather than just the examined scalar
* within `skeet-publish`:
    * [x] In publisher, publish the following for each `PublishedList` at, for example, `v3-quality-7d:statistics` as a json object:
        * start/end of interval covered (so, absolute start/end of the 7d period in this example)
        * count of examined images
        * count of images we eventually show (this is just the length of the list)
* within `skeet-feed`:
    * [x] Get the counts of images examined and shown, and the interval given, and use these to create the "(400,000 images checked over past 2 days, of which 46 (0.01%) match what we are looking for)" text. (With the firehose-slice fallback, read stats for the list fallback actually served — the served window, e.g. website `quality-4w` widening on degrade — not a fixed `quality-7d`/"2 days".)
* [x] refactor any existing `count` methods in other `skeet-store` traits to live in the `Statistics` trait
* [x] add a small internal consistency sanity-check test which can run locally and also as an integ test against staging and production which checks:
    1. for the "of which X (Y%) match" text, from which we extract X
    2. there are X images shown in page i.e. there are X `img` elements in the page under `<div class="grid">`
    * (committed deliberately red: the banner reports `stats.found` = the published list length, which still counts candidates the publisher's existence probe found deleted, while the grid renders only the live-filtered images, so `found > shown`.)
* [x] make the banner/grid consistency a publisher-owned invariant (keeps `skeet-feed` a dumb formatter), turning the red test above green:
    * add an `exists` count to `ListStatistics` alongside `found` = the number of *live* items (`image_url_exists && skeet_id_exists`, same meaning as in the published list) in the list the publisher writes, computed at publish time via the **shared `is_live` predicate** in `skeet-publish` (reuse the one in `source.rs` so the publisher's count and the feed's render filter can't drift)
    * `skeet-feed` displays `stats.exists` (and the percentage `exists / examined`) verbatim instead of deriving a count — the feed keeps filtering `is_live` to render (the list stays a superset for appraise), but no longer does any banner arithmetic
    * add a **publisher-side** consistency test: the `exists` written into a list's statistics equals the number of `is_live` items in the list it just wrote
    * note: `replace` + `write_statistics` aren't atomic, so a reader can briefly see a list and stats from different cycles (same self-healing race as `refreshed-at`) — acceptable
* [x] once `skeet-feed` deployed and not using it anymore stop creating/publishing `v3-examined-count` — also retire `estimate_processed`/`SAVE_RATE_PERCENT`, the `saved × 500` guess the real measured count replaces.
    * also removed the whole read path it fed (`ExaminedCount`, `PublishedImagesSource::examined_count` + fallback plumbing) and `Statistics::count_scored_images` (the `scored` input to the guess), all now dead.
* implement the end of page experience in steps:
    1. [x] Implement the "You've reached the end of the images found so far! Next one expected to be found in X hours" first in the simplest way, by doing a feed-side calculation of what X hours is predicted to be, based on assuming a uniform arrival time over the period. Make there be a JS countdown timer which reloads the page when this time is reached. This countdown timer should probably be at the top as well, as a new sentence after "match what we are looking for"
    2. [ ] Then, in the publisher, do the proper maths i.e. treat this as a Poisson process and calculate confidence intervals of when, for each bucket of time (48h, 7d, 1y) and given current time, over what period of time we have a 95% chance of seeing another example of what we are looking for. Add this to the `:statistics` JSON as a few keys showing lower,middle,upper timepoints predicted. This prediction should take account of all arrival times seen during the period of interest. This should extend the `Statistics` port to implement these stats as needed.

        **The model (deliberately kept simple — plain Poisson, no Bayesian/Lomax).** Throughout this step `now` means the **end of the current interval** (`interval_end`), not wall-clock call time — that's the origin we predict forward from. Treat matches as a homogeneous Poisson process with rate `λ = count / window`. The only fact needed: the time from *now* to the next match is `Exponential(λ)` (memoryless — independent of how long since the last one). The exponential CDF is `P(seen within t) = 1 − e^(−λt)`; invert it for "time by which there's a probability `p` of a match":

        ```
        T(p) = −ln(1 − p) / λ
        ```

        Everything we publish is this one formula at different `p`:
        * `lower`  = `T(0.025)` = `−ln(0.975)/λ` ≈ `0.025/λ`
        * `middle` = `T(0.5)`   = `ln(2)/λ`      ≈ `0.69/λ`  (the median wait)
        * `upper`  = `T(0.975)` = `−ln(0.025)/λ` ≈ `3.69/λ`

        Emit these as `now + T(p)` absolute timepoints (or the waiting durations — pick one and be consistent). Step 1's `window/count` is the **mean** wait `1/λ`, which is slightly longer than the `middle` median — same model, different summary, not a bug.

        Decisions baked in:
        * **Drop parameter uncertainty in λ** (the ~15% from N≈46) and anything needing a conjugate prior. Process randomness (the exponential above) is the dominant, honest uncertainty; the rest is what required the scary distribution.
        * **Zero-count fallback** (replaces what the prior bought us): if a window has 0 matches `λ = 0` → infinite/undefined wait, so don't predict — emit `None` and let the feed say "not enough recent data" rather than show a countdown.
        * **Clock:** estimate `λ` from `discovered_at` of items that survive the `is_live` filter, *not* `original_at` (post time). `discovered_at`→classify→`is_live`→publish-cadence is what actually drives a new image appearing on reload; the wrong clock makes the countdown systematically under-run reality (and the publish cadence is a floor on "next visible arrival").
        * **Library:** `T(p) = −ln(1−p)/λ` is a one-line `f64` (comment it as "exponential inverse-CDF"); that's more transparent than reaching into a crate and isn't really hand-rolling an algorithm. `statrs::Exp::inverse_cdf` is a fine alternative if preferred.

        Known limitations (acceptable for now, don't chase):
        * **Night-time bias on the 48h bucket.** Real arrivals follow day/night and weekly cycles, not a constant rate. At ~3am the flat-rate model over-promises and the countdown can expire with nothing new; the 7d/1y buckets average this out. The fix (a time-of-day / inhomogeneous rate) is real work — defer unless the 48h countdown proves annoying in practice.
        * **1y rate is stale.** Most data but the rate drifts as Bluesky grows; the year-window rate is the least relevant to "right now".
        * Optional sanity check: compare variance-to-mean of per-hour counts; if `≫ 1`, arrivals are clustered (a festival/burst breaks independence) and the intervals should be read as too narrow.
    3. [ ] Update the feed website to use the middle timepoint as it's predicted time of next arrival, and to show the 95% confidence range (in text). Phrase the `upper` bound as the "95% chance by" point. When the publisher emitted no prediction (zero-count fallback above), show "not enough recent data" instead of a countdown.

### Observation (2026-06-28): R2 Class B operations rose noticeably after adding Statistics storage

Since this slice added the `prune_stats_v1` table — written once per minute by the pruner (`StatisticsPersister`) and read by the publisher to compute the banner — R2 class B (read) operations went up noticeably.

The diagnostic data is already collected: the `r2.operations` OTel counter (`R2MetricsWrapper`) is labelled by `table`, `kind` (`data`/`manifest`/`_versions`/`_indices`), `operation`, `r2_class`, and `cli`. Confirm/attribute each hypothesis with, in Grafana/Mimir:
* `sum by (table, r2_class) (rate(r2_operations[…]))` — should show `prune_stats_v1.lance` as the new class-B contributor.
* add `kind`: `data` spiking ⇒ Hypothesis A (fragment scans); `manifest`/`_versions` ⇒ Hypothesis B/C (manifest churn).
* split by `cli`: `skeet-publish` dominating ⇒ A/B; `pruner` ⇒ C.
* the `prune_stats` `fragment_counts` gauge — a sawtooth peaking ~60 each hour corroborates A.

The root cause in all cases is the **minute-ly single-row append**, but the dominant spend is read-side amplification, not the writes themselves. Hypotheses below are ranked by suspected impact and are **unconfirmed** pending the metric split above.

##### Hypothesis A: Publisher re-scans `prune_stats` 7× per publish over un-compacted single-row fragments — UNCONFIRMED (suspected primary)

**Background:** `publish()` calls `prune_stats_for_interval(...)` once per spec (`publisher.rs:301`); production publishes 7 specs (`infra/k8s/skeet-publish-deployment.yaml`). The pruner appends one row/minute, each its own single-row Lance fragment. The optimise CronJob runs hourly (`infra/k8s/optimise-cronjob.yaml`: `0 * * * *`), so up to ~60 single-row fragments accumulate between compactions. The BTree index on `interval_start` (`schema.rs:72`) only covers fragments as of the last optimise run, so the fresh tail — exactly what recent-window queries need — is flat-scanned, one class B `get` per fragment data file. Publishes are frequent because the gate (`RELEVANT_TABLES`) fires on `scores`/appraisal changes and `live-refine` writes scores continuously. Cost ≈ `(publishes/hour) × 7 × (fragments accumulated)`.

**How to prove/disprove:** the `kind="data"`, `table="prune_stats_v1.lance"`, `cli="skeet-publish"` slice of `r2.operations` should dominate the increase and track the `fragment_counts` sawtooth.

##### Hypothesis B: `version_snapshot()` reads the new table's manifest every tick — UNCONFIRMED (constant floor)

**Background:** `publish_if_changed` calls `version_snapshot()` every `interval_secs` (default 60), which calls `table.version()` for *every* table including `prune_stats` (`versions.rs:18-25`). With store-wide `read_consistency_interval(Duration::ZERO)` (`open.rs:25`) each `version()` forces a fresh manifest resolve (class B `get` of the latest manifest + class A list). Adding the table costs a flat ~1,440 extra class B/day independent of whether anything publishes.

**How to prove/disprove:** `kind="manifest"`/`_versions`, `cli="skeet-publish"` should show a steady ~1/min baseline even across idle ticks (no publish).

##### Hypothesis C: The minute-ly appends' own manifest resolves — UNCONFIRMED (smallest)

**Background:** each `add()` (`statistics.rs:68`) under strong consistency does a checkout-latest (class B manifest read) before writing fragment/txn/manifest (class A). ~1,440 class B/day from the pruner. Modest next to A.

**How to prove/disprove:** `cli="pruner"`, `table="prune_stats_v1.lance"` class B ≈ writes/day.

##### Considered and rejected: eagerly optimise the `prune_stats` BTree index after each append

Mechanically trivial (`OptimizeAction::Index` is incremental and already runs per-table in `maintenance.rs`), but it doesn't address the bottleneck and likely makes it worse. A scalar BTree narrows *which rows* but doesn't avoid reading them: `prune_stats_for_interval` sums `skeets_seen`/`images_examined`/`images_saved` (`statistics.rs:110-127`), so execution still *takes* those column values from the data files — with single-row fragments every matching row is its own `get` regardless of the index. The recent-window queries match nearly all the fresh fragments anyway (index pruning only skips *old*, already-compacted, already-cheap fragments). On top of that, reading the index adds `_indices/` class-B gets and optimising every minute adds class-A index-delta churn (a second small-files problem). The lever is fragment count, not index coverage.

##### Tasks to fix

* [ ] **Collapse the publisher's 7 per-spec reads into 1.** Read the widest window (`quality-1y`) once and aggregate the narrower windows in memory (rows are already per-interval), instead of 7 independent scans (`publisher.rs:291-308`). Drops A by ~7×. Optionally cache for the tick.
* [ ] **Batch / coarsen the appends (addresses the root cause).** Buffer per-minute `ContentCounts` in `StatisticsPersister` and flush a single multi-row batch much less often (e.g. hourly), or record coarser (hourly) buckets at source. One fragment + one manifest bump per flush instead of ~60. Cuts A's fragment count and C's write churn proportionally; no query/data-model change.
* [ ] (stretch) **Relax consistency for the stats read path.** `read_consistency_interval(Duration::ZERO)` is store-wide and forces a manifest resolve on every op (drives B). Stats tolerate minutes of staleness; making this read path eventually-consistent would remove B's per-tick manifest reads — but the setting is store-wide today, so this needs a way to scope it.
* [ ] (consider) **Question the store choice.** These counts are already emitted as OTel counters to Mimir, and the publisher already writes a per-spec `:statistics` JSON to redis. Serving the examined-count from redis (or folding it into that JSON) would avoid LanceDB entirely for this hot path. Larger change — only if batching + read-collapse don't bring it down enough.
