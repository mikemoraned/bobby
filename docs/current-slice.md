# Current Slice: Slice 8 — Minimal qualitative scoring on top of Envelope filtering

### Context

What we have effectively been doing so far is doing a bunch of quick checks to exclude 'obviously' non-matching skeets. So, biasing towards checks which allow a small %-age of positives through which may be wrong and exclude a large number of negatives.

Now that we have a small (sub 1%) amount coming through, we can apply some more expensive operations on the 1%.

### Target

* a new `skeet-scorer` crate which applies a score to an image, between 0.0 (worst) and 1.0 (best), that captures how closely an image matches our intent
    * it should use an LLM to do this scoring
* a `skeet-feed` which shows the top N skeets, ordered by score, best first

### Tasks

* [x] make `ImageId` a unique key with an index for correctness and performance
    * [x] improve performance of `get_by_id` by added an index:
        1. Add a BTree scalar index on the image_id column e.g. `table.create_index(&["image_id"], Index::Auto).execute().await?` (should choose BTree behind-the-scenes)
        2. After any writes to the table, call `table.optimize(OptimizeAction::All).execute().await?` to keep the index current
        3. On queries that lookup by `image_id`, add `.limit(1)` and `.select()` with only the columns needed
    * [x] update `ImageId` so that it acts like a content-addressable hash
        * i.e. when it is created it shouldn't just be a random unique uuid, but instead should be a hash (e.g. md5) of the byte contents of the image
    * [x] update `skeet-find`/`skeet-store` so that, when it wants to save an image it has found, it first checks to see it does not already exist, based on id

* [ ] fixes / refactorings:
    * [x] the status summary for `find` appears to be wrong. `rejected` appears to be a count of rejected reasons and not rejected images. It should always be a count of rejected images with the invariant that `images = saved + rejected`. See example below of this being broken:
    ```
    00:03:03 ⠸ skeets: 1483 | images: 183 | saved: 0 (0.0%) | rejected: 281 (FaceNotInAcceptedZone: 3 [1%], FaceTooLarge: 1 [0%], FaceTooSmall: 40 [14%], TooFewFrontalFaces: 169 [60%], TooLittleFaceSkin: 8 [3%], TooManyFaces: 25 [9%], TooMuchSkinOutsideFace: 14 [5%], TooMuchText: 21 [7%])
    ```
    * [x] we shouldn't be passing secrets as command-line variables, but instead as ENV variables.
        * Some rust rules may need updated to allow this.
        * What we want is:
            * secrets are passed as env variables
            * we should probably use `op run` here which can contain a file like `foo.env` which contains a mapping of needed ENV variables to the secrets path.
    * [x] I've been getting connection errors, which may be triggered by my underlying home connection being flaky, but reveal opportunities for more robustness
        * [x] make firehose connections more robust by randomly choosing an endpoint + setting connect and recieve timeouts after which we give up and retry on a different endpoint
        * [x] some problems in uploading seem caused by multi-part uploads which timed out
            * this is probably not fixing the underlying issue, but create a small tool to clear them from R2
        * [x] it looks like it may be possible to get a thumbnail version of an image rather than a full one. We should use this if possible as we'll reduce download, analysis, and upload time
        * [ ] more generally make more robust to errors when uploading remotely
            * [x] refactor firehose into two stages (keep all current behaviour as-is):
                * `filter`: this is the bulk of it which finds and identifies good image candidates
                    * this is what talks to jetstream, downloads images, applies face/skin/text detection and produces a candidate image
                * `save` : this what takes the image found and saves to the store
                * connect these two parts with a pipe/channel where the output is an ImageFound message
            * [x] now introduce the idea of a `fallback` local store for when a remote image save fails; this is analogous to a "dead-letter queue"
                * [x] the `save` stage should still attempt to save remotely, but when there is a failure, it instead saves it to the local `fallback` store
                    * both of these stores use `SkeetStore`; one is remote on R2 and one is local
                    * a new `--fallback-local-store` param should be introduced to `find-r2` and cli so that it use a local `fallback` dir
                    * update `Status` so that it has a count of `saved-remotely` and `saved-fallack` whilst still keeping track of overall `saved`
                * [x] add a new `redrive-r2` CLI bin to `skeet-store` which can be used to reconcile the local store with a remote one by attempting to upload anything that exists in the local `fallback` store but not in the remote one in R2
                * [x] extend `redrive-r2` and `skeet-store` mod so that:
                    1. when it finds an image in fallback store that already exists in remote store, it does a deeper comparison where it asks ImageRecord (or similar) to a deep equal on the bytes stored. It should show whether that worked or not
                    2. when it has verified it definiyely exists remotely with exact same content in step 1, it should delete that image from the local fallback store
                        * this will involve extending `SkeetStore` to have a `delete_by_id` method, and associated tests
        * [x] increase reliability to remote R2 stores:
            * Theories on R2 timeout errors (2026-03-22):
                1. **Per-request HTTP timeout too short for large range reads** — lance-io downloads 5–13 MB ranges; with the default object_store per-request timeout (~5 s) and a variable home connection this easily times out. lance-io retries 3× internally regardless of `client_max_retries`.
                2. **R2 rate limiting under concurrent load** — a single `exists()` call can trigger 300+ iops (visible in lance execution logs). Confirmed no 429s in R2 dashboard, but throttling may manifest as slow/dropped connections.
                3. **Lance dataset fragmentation** — each `add()` creates a new data fragment. Over time this fans reads across many small files, amplifying the probability of at least one range read timing out.
                4. **Stale multipart uploads** — confirmed cleared via `abort-multipart-uploads` (no outstanding uploads as of 2026-03-22).
            * TODOs:
                * [x] set generous HTTP timeouts (`timeout`, `connect_timeout`) in storage_options, and increase the save-stage channel buffer to 100 to take advantage of the decoupled save stage
                * [x] enable `lance_io=debug,object_store=debug` logging to see per-request HTTP timings
                * [x] auto-compact: trigger a compaction step after every N writes (configurable, default 100) to reduce fragment count
                * [x] add a `compact` CLI to force-compact the lance dataset on demand
            * LanceDB best-practice audit (2026-03-22):
                * Docs reviewed: [Storage Configuration](https://docs.lancedb.com/storage/configuration), [FAQ](https://docs.lancedb.com/faq/faq-oss), [Scalar Indexes](https://docs.lancedb.com/indexing/scalar-index), [Lance Object Store](https://lance.org/guide/object_store/)
                * Findings:
                    * **`client_max_retries: 0` is too aggressive.** The default is 10, and the retry timeout default is 180s. With the decoupled save stage, retries won't block the pipeline. lance-io retries 3× internally for incomplete responses, but `client_max_retries` governs the object_store S3 client layer (transient HTTP errors, 500s, etc). Setting it to 0 means any transient S3 error is an immediate failure.
                    * Table lifecycle (open once, reuse), index choice (BTREE via Auto for high-cardinality string), `OptimizeAction::All` (Compact + Prune + Index) are all aligned with guidance.
                    * Concurrent writers on S3/R2 are safe: `object_store` defaults to `S3ConditionalPut::ETagMatch` which uses `If-None-Match: *` on puts — R2 supports this natively. A racing commit fails with `412 Precondition Failed` and Lance retries at the next version. DynamoDB commit store is deprecated in favour of this. Only relevant concern is efficiency under high contention (retry storms), not correctness — and we have a single writer.
                * [x] raise `client_max_retries` from 0 to 3 to handle transient S3/R2 errors without blocking the pipeline

* [x] correctness:
    * [x] write a SKILL which is invocable by `/add-to-blocklist` which:
        1. Asks user whether we want to use the fallback store or the remote store 
        2. Asks user for what they want to block. This can either be an image id or an at id. It should work out which it is by whether it starts with `at://` or not
        3. Asks user why they want it blocked, and remember as the `reason`
        4. Based on these answers:
            1. (Optionally) fetches image id details, if needed, to find the `at://` URL using appropriate Justfile rule for remote or fallback store (`image-metadata-dump-r2` or `image-metadata-dump-fallback `)
            2. runs Justfile rule `add-to-blocklist` for `at://` URL with `reason` given
    * [x] I'm not convinced the filtering by Adult content and "The author of this post has requested their posts not be displayed on external sites." is working properly.
        * Here are three examples that should have been filtered that I have added to blocklist:
            * "at://did:plc:4cg25zjw2wuqvnduwqgy7ozt/app.bsky.feed.post/3mhne43icps2l", "at://did:plc:4yqhj5inp67fgorcbewk5zfm/app.bsky.feed.post/3mhnexg5g3k2o" => should have been blocked as asks to not be displayed on external sites
            * "at://did:plc:vx76uzb6m2lvgh3kvbiagsg3/app.bsky.feed.post/3mhne4wdiuc2d" => should have been blocked as adult content
        * [x] we need to examine/fix:
            * [x] why weren't these specific examples fixed/blocked? (examine and apply fixes)
            * [x] is there something more fundamental here in that perhaps we are writing tests / blocking these in code that is not actually used in main firehose path?
                * suggest adding a stronger integ test (like `blocklist.rs` and `examples.ts`) which runs the real firehose code but with pre-loaded inputs (i.e. the blocklist json data) and asserts that nothing gets through that should be blocked
                * it may easiest to do this by first splitting the `filter_stage` into two sub-stages:
                    * `filter_meta_stage` : applies the blocklist filters which only require metadata about a Skeet
                    * `filter_image_stage` : applies the remaining filters which require accessing an actual image
                * our integ test then probably only needs to test the code in the `filter_meta_stage` and so avoids having to mock out things like image fetching etc
                * we should do this in a Red/Green/Refactor style i.e.
                    * if the theory is correct that we are not using the code we need in main firehose path, we should first prove that by implementing a failing integ test (including doing the refactoring to support this)
                    * then prove we've fixed it by running the (ideally unaltered) integ tests which should now pass

* [x] efficiencies / performance:
    * [x] the `SkeetStore` re-opens the images table each time it uses it (`let table = self.db.open_table(TABLE_NAME).execute().await?;`)
        * I suspect it doesn't need to do that and can instead keep it open. Have a look at the docs for `open_table` on recommended usage and see if it's recommended / allowed to keep it open.
    * [x] in `skeet-feed` in `handler.rs`, `feed` and `annotated_image` methods call `open_store` on each call. They probably don't have to, and could instead have a `SkeetStore` preopened on startup and saved in some context.
        * See https://cot.rs and related docs (https://docs.rs/axum/latest/axum/) for recommendations on how to do this.
            * For example it is typical in Axum apps (which cot uses) to hold global things like DB's in AppState; see https://docs.rs/axum/latest/axum/extract/struct.State.html

* [ ] minimal `skeet-scorer`
    * add a new table `images_score` which contains:
        * an `ImageId` as a key which is a foreign key to the `images` table for that image
        * an f32 score
    * we will use OpenAI here, accessed via Rust API's, as our content generator
        * even though we are using OpenAI in this initial pass, we should use Rust crates which are generic and allow other LLM's to be plugged in later
        * we will pass in OpenAI API keys from 1Password Dev access, `hom-bobby-openai-key`
    * we want to end up with a few clis:
        * [x] `train`: goes through all the images in `examples/expected.toml` and attempts to find a summary which gives a high score to the ones labelled `exemplar = true` and a low score to those `exemplar = false`
            * the output of this should be a list of instructions captured in a `model.toml` file, which capture the summary
        * [x] `rescore`: go through everything in `images` and assign a score; is allowed to overwrite the score in the `images_score` table
            * reads `model.toml`
        * [x] `live-score` : every minute, finds all images that have been added in past minute and which do not have a score, and assigns one
            * reads `model.toml`
    * [ ] add a config version to `model.toml`, and related structs (e.g. `ConfigVersion`), similar to what we have for `archetype.toml`:
        * [ ] add something similar to `ConfigVersion` (maybe `ModelVersion`) which captures a hash of the config as with `archetype.toml`, and is kept up-to-date in a similar way
        * [ ] update the scoring so that it attaches the config version to the score; this will require an update to the table version

* [ ] debugging helpers:
    * [ ] add a small `summarise` cli within `skeet-store` which:
        * connects to a store and, via `SkeetStore`:
            * counts how many entries there are in each main type of thing i.e. how many images, how many scores
            * counts how many images have a score
        * extract a shared helper for this within `SkeetStore` which creates a `SkeetStoreSummary`
    * [ ] add `summarise` and `summarise-r2` Justfile rules which run summarise against local and remote store
    * [ ] this same functionality should also be added to the homepage of the `skeet-feed` so that it shows a `SkeetStoreSummary`

* [x] updated `skeet-feed` to have two pages, which replace the homepage
    * [x] `latest` : this is the current page which shows the latest skeets received, regardless of whether they have been scored
    * [ ] `best` : same as latest except only shows those scored, and orders from best to worst
    * [x] homepage should have links to each of these
