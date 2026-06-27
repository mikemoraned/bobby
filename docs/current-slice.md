# Current Slice: Make statistics more visible / understandable

### Target

As of 27th June we say "(22,223,000 images checked so far)" on https://bobby.houseofmoran.io but this doesn't make it clear how few of these actually match the archetype.

We'd like to change this to say something like "(400,000 images checked over past 2 days, of which 46 (0.01%) match what we are looking for)". This should show human-readable numbers and days e.g. time rounded to nearest hour/day/week/month/year multiple, and percentages shown to a round two decimal places.

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
    * [ ] In publisher, publish the following for each `PublishedList` at, for example, `v3-quality-7d:statistics` as a json object:
        * start/end of interval covered (so, absolute start/end of the 7d period in this example)
        * count of examined images
        * count of images we eventually show (this is just the length of the list)
* within `skeet-feed`:
    * [ ] Get the counts of images examined and shown, and the interval given, and use these to create the "(400,000 images checked over past 2 days, of which 46 (0.01%) match what we are looking for)" text. (With the firehose-slice fallback, read stats for the list fallback actually served — the served window, e.g. website `quality-4w` widening on degrade — not a fixed `quality-7d`/"2 days".)
* [ ] refactor any existing `count` methods in other `skeet-store` traits to live in the `Statistics` trait
* [ ] once `skeet-feed` deployed and not using it anymore stop creating/publishing `v3-examined-count` — also retire `estimate_processed`/`SAVE_RATE_PERCENT`, the `saved × 500` guess the real measured count replaces.
