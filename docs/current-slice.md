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
    * [ ] I've been getting connection errors, which may be triggered by my underlying home connection being flaky, but reveal opportunities for more robustness
        * [x] make firehose connections more robust by randomly choosing an endpoint + setting connect and recieve timeouts after which we give up and retry on a different endpoint
        * [x] some problems in uploading seem caused by multi-part uploads which timed out
            * this is probably not fixing the underlying issue, but create a small tool to clear them from R2
        * [ ] it looks like it may be possible to get a thumbnail version of an image rather than a full one. We should use this if possible as we'll reduce download, analysis, and upload time

* [ ] minimal `skeet-scorer`
    * add a new table `images_score` which:
        * contains an `ImageId` as a key which is a foreign key to the `images` table for that image
        * an f32 score
    * we will use OpenAI here, accessed via Rust API's, as our content generator
        * even though we are using OpenAI in this initial pass, we should use Rust crates which are generic and allow other LLM's to be plugged in later
        * we will pass in OpenAI API keys from 1Password Dev access, `hom-bobby-openai-key`
    * we want to end up with a few clis:
        * [ ] `train`: goes through all the images in `examples/expected.toml` and attempt to find a summary which gives a high score to the ones labelled `exemplar = true` and a low score to those `exemplar = false`
            * the output of this should be a list of instructions captured in a `model.toml` file, which capture the summary
        * [ ] `rescore`: go through everything in `images` and assign a score; is allowed to overwrite the score in the `images_score` table
            * reads `model.toml`
        * [ ] `live-score` : every minute, finds all images that have been added in past minute and which do not have a score, and assigns one
            * reads `model.toml`

* [ ] updated `skeet-feed` to have two pages:
    * [ ] `latest` : this is the current page which shows the latest skeets received, regardless of whether they have been scored
    * [ ] `best` : same as latest except only shows those scored, and orders from best to worst
