# Current Slice: Slice 15 — re-introduce text-filtering to reduce costs / increase quality

### Target

As of 15th April, I am seeing a lot of low-quality skeets come through with text in them. We previously were applying text filtering but it didn't seem to be of much value, as it was only excluding a small %-age. It may be that were just lucky before and now it'd be more useful.

Now that we have an ability to manually appraised skeet images by quality we should use this to establish a test set.

So, what we want is:
* a manually-appraised set (200 should be enough) of images
* text-based pruning re-applied, perhaps differently to before
* a measurement of precision on this test set before/after pruning by text

### Tasks

#### Cleanups

* [x] I added `tokio-console` support but I've not really used it. I originally added it because I thought it was more like a local telemetry viewer than async debugger. So, generally useful, but not for most of what I've been doing. TLDR: support for it should be removed, and dependency can be deleted.

#### Bugs

* [ ] is auth login actually working for github admin when deployed? (it does work locally i.e. when I go to localhost)
    * [x] if I go to https://bobby-staging.houseofmoran.io/admin I get the following:
        * response:
        ```
        The redirect_uri is not associated with this application.
        The application might be misconfigured or could be trying to redirect you to a website you weren't expecting.
        ```
        * url contains:
        ```
        &redirect_uri=http%3A%2F%2Fbobby-staging.houseofmoran.io%2Fauth%2Fcallback&scope=read%3Auser
        ```
        * I think this should be `https` not `http`?
    * [x] I think I am now seeing issues caused by using a `MemoryStore` and the fly.io machine stopping; this is not too surprising. To fix this:
        * [x] before making changes, upgrade cot.rs to 0.6 (we are on 0.5); apply any fixes for breaking changes
        * [x] do a bit of research into things like [tower_sessions](https://github.com/maxcountryman/tower-sessions) and [axum-login](https://github.com/maxcountryman/axum-login); my preference is to re-use as many standard thirty-party code as possible for auth-related activities on the assumption it has been well-tested
        * [x] what I want to end up with is:
            * when `--local-admin` is enabled, the Appraiser is `Appraiser::LocalAdmin`; no persistent session store should be needed, other than one in-memory
            * when not enabled:
                * Appraiser is derived as now i.e. a github login is done as now
                * The sessions, or any other state, is kept in a redis DB
                    * an upstash.com redis has been created, and the URL is 1Password in the `bobby-upstash-redis-tcp-url` entry
    * [ ] deploy to https://bobby-staging.houseofmoran.io/ do a manual test to check that https://bobby-staging.houseofmoran.io/admin works:
        * [x] the deploy to fly.io failed on startup because first, the secret was malformed (I fixed that) but then because we've not compiled in support for tls: "failed to create Redis session store: PoolCreation(Config(Redis(can't connect with TLS, the feature is not enabled - InvalidClientConfig)))". We can fix this, but first we need a failing integ test so we catch these sorts of problems earlier:
            * [x] create an integ test which uses a testcontainers redis setup (https://testcontainers.com/modules/redis/?language=rust) to create a local redis for use in tests, that we can attempt to connect to over TLS and see it fail
            * [x] add TLS support and see it pass
            * [x] re-enable redis in fly.io by setting `--use_redis`
        * [ ] login, do some admin e.g. appraising some images
        * wait a few hours, don't reload page
        * [ ] do some more admin, without logging-in again, and check still works

#### Manual Appraisal

* [x] extend feed admin pages to show overall counts of number of appraised skeets and images, on respective views
* [x] manually appraise 200 images
    * there was actually already 354

#### (Imperfect) Precision/Recall measure

* [x] write a small CLI in `skeet-prune` called `eval` which:
    1. finds all images in a store that have been manually appraised into a particular Band i.e. ignore anything not manually appraised
        * this may involve adding support to `SkeetStore` for this
    2. map the Band for an image to a binary `should_be_pruned` variable:
        * Band = Low, then should be pruned, `should_be_pruned` = true
        * Band = anything else, then may be allowed, `should_be_pruned` = false
    3. fetch the information for these images we want to assess, and for each, run them through a classify pass, where we collect whether an image would have been pruned or not
        * read-only: no store updates, just re-run classification on stored images
        * add a batch loading method to `SkeetStore` (e.g. `get_by_ids`, batch size ~10) to load images without pulling entire store into memory
    4. do precision/recall evaluation by taking `should_be_pruned` as the actual, and whether it was pruned in step 3 as the prediction
        * output a summary text table to stdout (TP, FP, TN, FN, precision, recall, F1)
        * also output same data as a CSV file (via `--output-csv` flag) so it can be checked-in and compared across runs
        * note that as overall measures these are skewed, as the only images that have been appraised are the ones that previously had not been pruned. so we are biasing towards only examining that subset, and not the wider unknown set that was never seen by a person. this is ok, as we are using this here as a way to see if text-detection can be a narrower more precise way to exclude images. We are aiming to measure an increase in precision and no loss of recall, and this is measurement method is sufficient for that.

#### Re-introduce text-based filtering as an optional filter

* [x] go back through commit history and bring back the text-detection crate contents (see commit `92a72bfc2f7095eff4601fea40f8c271044ccb0a`). don't yet hook it into any classification i.e. we won't use it for real yet
* [x] run mutation-testing on this, to flush out any testing gaps. also migrate any tests to prop-based style
* [x] make classification methods configurable by making it so that we can optionally use text-detection, but face-detection and skin-detection are on by default.
    * note this means we *don't* enable usage of text-detection as a side-effect of defining something in [config](../config/prune.toml) like `text_area_pct`
    * instead we should make it so that we can separate detailed config (e.g. `text_area_pct` and `min_face_area_pct`) from whether a particular category is enabled.
    * we can do this by adding a `HashSet` of `RejectionCategory` to `PruneConfig`, `categories` which has a default but can be overridden: when `PruneConfig::from_file` is called, it can be passed an `Optional` `categories` argument which can override the default. This should also effect the version hash.
    * then when setting things up, we can use this list of categories to decide what to initialise
    * we should always put config like `text_area_pct` in [config](../config/prune.toml) as we can now control whether something is enabled separately

#### Evaluate text-detection

* [ ] using above capabilities, do two runs of `eval` one with defaults (no text detection) and one with text-detection enabled and compare precision/recall
    * [x] make `eval` have an arg which it passes to `PruneConfig` to decide which `RejectionCategory`'s to use
    * here are the results:
        * default (no textdetection):
        ```
        Evaluation against 457 manually appraised images

        Confusion matrix (positive = should be pruned):
                        Predicted: keep Predicted: prune
        Actual: keep    137             0
        Actual: prune   320             0

        TP=0  FP=0  TN=137  FN=320
        Precision=0.000  Recall=0.000  F1=0.000
        ```
        * with text-detection:
        ```
        Evaluation against 457 manually appraised images

        Confusion matrix (positive = should be pruned):
                        Predicted: keep Predicted: prune
        Actual: keep    130             7
        Actual: prune   67              253

        TP=253  FP=7  TN=130  FN=67
        Precision=0.973  Recall=0.791  F1=0.872
        ```
    * **Conclusion**: text-detection is highly effective — 97.3% precision means almost no good images are wrongly pruned (only 7 of 137), and 79.1% recall catches the majority of low-quality images.
        * [ ] Recommendation: enable text-detection in the default category set for production.
