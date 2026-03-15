# Purpose

Find selfies people take of themselves with physical landmarks (famous buildings, monuments, places like the Eiffel Tower).

I've already done this once before, but with Twitter: https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/. The purpose is to recreate this but using bluesky instead of twitter, and using modern technologies. See [original repo](https://github.com/mikemoraned/selfies) for context.

The original project scanned Twitter's firehose, applied face detection (via OpenIMAJ), and looked for selfie-like compositions (face in foreground border, landmark in background). It had a ~0.1% hit rate and encountered challenges like porn, screenshots, and false positives from inanimate objects resembling faces.

# Prerequisites

Install required system dependencies via:

```
just prerequisites
```

# Methodology

We're going to follow a [Walking Skeleton](https://wiki.c2.com/?WalkingSkeleton) approach where we will incrementally deliver slices end-to-end

## Constraints, trade-offs and technology choices

* Where possible, all code should be written in Rust.
    * It's acceptable to use a non-Rust language or toolset for the purpose of getting the bluesky firehose data, including images. However, once an image is fetched everything else must be in Rust.
    * It's ok to use non-Rust ML models.
* Use existing models, or Rust libraries, for face-detection and landmark identification.
* Please use Burn (https://burn.dev / https://github.com/tracel-ai/burn / https://burn.dev/get-started/) for running ML Models
* It may not be possible to process images at the same rate as they are received (line-speed). This is totally fine. In this case we can adopt a sampling approach where we sample from the stream at the rate at which we can process images.
    * However, we should aim to do the really simple parts inline with receiving a message. For example, identifying if a message contains an image is likely something we can do at line-speed.
* any command-line invocations should be captured in the Justfile

## Invariants / Style

* models:
    * any models should be downloaded to `models` dir
    * when adding a new model, add a short doc to the `docs` dir summarising where it comes from, what it does, and why we are using it
* commenting / docs:
    * the focus should be on making the code itself self-documenting as opposed to requiring extra commenting
    * comments can still be added, but should be focussed on not something already covered by the code.
    * if substantive documentation is needed, it's better to create a separate markdown doc in `docs` dir.
* stability / quality: where possible we should follows these protections:
    * don't use `-pre` versions of dependencies
    * don't use direct git versions of dependencies
* exception: `jetstream-oxide` (pre-1.0) is allowed as it is the best available Rust client for Bluesky's Jetstream

## Rust specifics

* see above general guidance about comments. However, if comments are needed, please follow [rust doc guidelines](https://doc.rust-lang.org/stable/rustdoc/write-documentation/what-to-include.html)
* use external crates for core things like datetimes etc; don't write our own versions of these
* when multiple crates share a dependency on the same crate, pull this dependency out to a shared definition at workspace level
* always use latest rust version and edition where possible, but do not use rust nightly
    * specify the rust version in `rust-toolchain.toml` and the edition in `edition` in `Cargo.toml`
* usage of `unwrap`:
    * this is denied by default. if absolutely needed, please annotate with `#[allow(clippy::unwrap_used)]` and give a justification
* always apply `just clippy` after completion of each todo we complete
* where possible we should:
    * follow the [NewType](https://doc.rust-lang.org/rust-by-example/generics/new_types.html) idiom e.g. we should avoid having any bare Strings or f32's.
    * use types rather than untyped arrays. 
        * For example, when passing images, use things like `DynamicImage` or similar, instead of using an array of bytes.
    * where there is a possibility of something being missing, we should capture that as an Option::None, or a Result::Err
        * use Option::None when the item missing leaves the overall sub-system valid i.e. if it is expected or allowed for this to happen
        * use Result::Err when it represents an invalid state. In this situation the caller should call the method with `?` and consider if the error is significant enough that the program should stop.
* error representations:
    * errors should use structured Enums to represent the different causes of the error. Use [thiserror](https://docs.rs/thiserror/latest/thiserror/) for this.
* logical structuring:
    * roughly-speaking, anything that is a different kind of thing (e.g. a schema for a message) or a different layer (e.g. core message routing or image labelling) should live in it's own module, and have it's own tests.
    * any models that are used across the workspace in multiple crates should live in a separate `shared` crate which they depend on. The model structs etc can live in the `lib.rs` of that crate.
* testing:
    * for each core piece of functionality, it should have an associated inline unit test
    * for anything that requires multiple parts to prove it works, we should have an integ test
        * for this, we likely want to capture a real (but small) dump of firehose data to use
    * if possible our tests should be high-level and assert invariants rather than bespoke individual examples. in other words, we should use tests that have a high leverage between number of lines of test and breadth of behaviour tested
        * something like https://docs.rs/quickcheck/latest/quickcheck/ may be useful here
* command-line apps:
    * all config parameters should be passed explicitly as named command-line parameters e.g. `--long-form FOOP`
    * parameters *must not* be passed via environment variables other than things like `RUST_LOG`

## Python specifics

* always manage dependencies and install packages using `uv`

# Slices

## Slice 1: A random local feed

* [x] create a local skeet-store
    * sits in `skeet-store` crate and uses a LanceDB table called `images_v1`
    * this is stored as [Lancedb](https://lancedb.com) tables
    * the schema of `images_v1` table has:
        * `image_id` — globally unique UUID (v4)
        * `skeet_id` — bluesky AT URI of the skeet (can be duplicated across table, as one skeet may have multiple images)
        * `image_data` — the actual image stored as PNG bytes (LargeBinary)
        * `discovered_at` — UTC timestamp (microsecond precision) of when we first saw it
        * `original_at` — UTC timestamp (microsecond precision) of when the skeet was posted

* [x] create a skeet-finder which:
    * [x] listens to the live bluesky feed (via `jetstream-oxide`, filtered to `app.bsky.feed.post`)
    * [x] finds any which have images (checks `app.bsky.embed.images` and `recordWithMedia` embeds)
    * [x] randomly selects one image with 1% probability
    * [x] downloads images from Bluesky CDN and saves to the `images_v1` table
    * run via `just find` (store path defaults to `store`)

* [x] create a skeet-feed which:
    * [x] find all unique skeets from the `images_v1` table
    * [x] surfaces these just as a web-page which shows embedded skeets i.e. no actual Bluesky feed needed yet
        * uses [cot.rs](https://cot.rs) with Bluesky's embed.js for rendering skeet cards
        * reloads skeets from store on each page load
    * run via `just feed` (serves on http://127.0.0.1:8000/)

## Slice 2: finding faces

We're now going to start using some real models to find and detect faces.

* [x] update skeet-finder so that, instead of randomly selecting one image, it:
    * [x] only allows through images which contain at least one face. This face must be detected as being face-on i.e. side-profile faces are not allowed.
        * the "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx" ONNX model is one we've previously used which may be suitable here
        * document any model choices in `doc` dir
    * [x] matches to an archetype where the face is of a single person, and that persons face sits in one quadrant of the image.
        * this matching should be captured in a Archetype enum, which should be saved as an extra column of the images table
            * this is a backwards incompatible change, so table should now be name `images_v2`
        * see examples dir for example images which you should capture in a test:
            * examples/eno7kayhhljgvgwc7ttdoojx_3mfev3xjylk2w_0.png : Archetype::TOP_RIGHT
            * examples/jbbneqrt2fxcij3kjwxdu54m_3mfev4a57a22u_0.png : Archetype::BOTTOM_LEFT

* [x] to aid in debugging:
    * [x] annotated images:
        * [x] create an ability to create an annotated image out of each image found which shows:
            * the bounding box of the face (in red)
            * cross-hairs from side of image to borders of bounding box (in red) centred on centre of bounding box
        * [x] do a new version of store which extends the schema to v3 and adds a new column which is the annotated image
        * [x] extend the feed website so that it shows a table with three columns:
            * the unique id
            * the annotated image
                * there should be a new endpoint created which is `/skeet/{image_id}/annotated.png` which is the annotated image that be shown as `<img>`
            * the embedded skeet
    * [x] add a cli bin to the store crate which allows an image to be exported to a file, indexed by unique id
    * [x] problem: `list_all` in `SkeetStore` is very expensive to use, memory-wise, as it loads all images eagerly, and using this from `skeet-feed` means all images are loaded into memory on each request. We don't need to do this.
        * [x] introduce a new `StoredImageSummary` struct which contains all the same things as `StoredImage`, except for the actual image + annotated images.
        * [x] populate this `StoredImageSummary` in a new `list_all_summaries` method by running a query on the LanceDB table which only fetches the needed fields
            * watch out for continuing to try to share Arrow code where possible
        * [x] update `skeet-feed` to `list_all_summaries` as `StoredImageSummary` should contain everything it needs
        * [x] to avoid causing the same problem when the page is subsequently loaded in the browser, as all images will then be fetched:
            * [x] update the feed to always show a limited selection of most recent images
                * it should show the most recently-found 50 skeets

* [x] ignore faces of wrong size, position or number:
    * [x] regularize examples and tests, so that they are driven by config:
        * currently, we have tests like `example_top_right_face` which are very specific and embed the paths to the example files directly. The intent is to make tests be driven by config and appear as separate tests in the runner. So, for example, if I have a TOML config file which lists a bunch of examples, I'd like to have a separate tests each of which tests a particular *aspect* of these examples, but which each example appears as a separate executed test in the runner output. The intent is that I can drive tests by examples whilst making it appear like they were written as individual tests. So, I'd probably like to move to something like https://crates.io/crates/libtest-mimic which is more config-driven. 
        * [x] alongside examples, create an `expected.toml` (TOML format) file, which captures, for each image in that dir:
            * it's path
            * the expected Archetype enum wrapped in an Option. 
                * so, for example, if an image shouldn't be matched to any Archetype, it should be Option::None, but if it should, for example `examples/eno7kayhhljgvgwc7ttdoojx_3mfev3xjylk2w_0.png` it should be Option::Some(Archetype::TOP_RIGHT)
                * make this easy to update manually
        * [x] update existing tests to be driven by this config instead
    * [x] ignore faces that don't take up enough or too much of the image:
        * we want to ignore tiny faces that make up a small %-age of the area of the image and faces which dominate
        * we likely want to make this tunable/change-able as opposed to hard-coded. So:
            * [x] add an archetype.toml file in `skeet-finder` and an associated config type which captures the %-age upper and lower threshold as a float    
            * [x] set the %-age min threshold to 10% and max to 60%, and update tests. Examples:
                * `examples/064b26e2-550a-4925-9bd1-aa26d68b1742.png` should be filtered out because the face is too small
                * `examples/1de8f881-78be-4a89-8155-f85e5543b342.png` should be filtered out because the face is too large
            * [x] when we are classifying and/or ignoring an image we need to have a list of Enum of failure reasons that say why it was ignored (there may be multiple reasons)
                * for example: Reason::FaceTooSmall, Reason::FaceTooLarge
                * this should be added as a config item to the `expected.toml` file saying why an image is filtered
                * [x] as part of enabling tuning of these thresholds, we should have a diagnostics CLI, `classify_examples`, which, for each image in examples dir, outputs the current classifications and the underlying parameters used e.g. whether it is frontal or not, what %-age of image is the face etc
        * [x] add a version field to the config, and update the `images` table schema to record which version of the config was used to capture the image
            * [x] the version field should be automatically generated e.g. take all config values, sort them, and hash the result to a small string
                * obviously the version field should not be part of the hash
            * [x] add a failing test which checks a hard-coded version against actual
    * [x] extend feed website to show the matched Archetype as a column in the feed summary
    * [x] make Archetype matching more strict i.e. faces that sit in the middle of the image shouldn't match to any Archetype and should have a None value.
        * an example is `examples/a5d59a02-b46e-478b-ac46-801f67b9ac40.png` which is too much in the centre
        * a suggested way to model/determine this is to:
            * define 5 zones in an image (this replaces the Quarter enum):
                * 4 quarter Zones which cover 50% of the image and cover top-left, top-right, bottom-right, bottom-left
                * a central Zone which is same size as each of the quarter zones but is centred on the middle of the image
            * map a detected face to a Zone by measuring %-age overlap with each of them, and choosing max overlap
            * if an face maps to the Central Zone, then it doesn't match the Archetype (maps to None) and if it's one of the quarter Zones, then it maps to the associated Archetype
        * note that this means `examples/jbbneqrt2fxcij3kjwxdu54m_3mfev4a57a22u_0.png` is, for now, excluded as face is in centre. For now, we accept this and may add more complex categorisation
    * [x] ensure we exclude images with multiple faces
        * for example `examples/43344f90-e12f-4c06-bd54-ec7fb51211e1.png` should be exluded with an enum value of "TooManyFaces"
        * [x] update expectations
        * [x] fix code to ensure we reject this image


## Slice 3: False positive: Removing pron

* [x] apply refactorings:
    * [x] the code in `main.rs` in `skeet-finder` is too complicated. Break it into two sub-modules:
        * one that purely handles the Bluesky or JetStream-specific work of getting any skeets that contain images, and downloading these images
        * another that handles the calling of code to specfically classify images and save them to the store

* [x] apply cosmetics
    * [x] rather than being primarily a log-style output, change `skeet-find` `main.rs` so that it uses https://docs.rs/indicatif/latest/indicatif/ to produce a persistent summary line that contains:
        * a continuous spinner to show it is alive
        * how long it has been running
        * number of:
            * skeets seen
            * images seen
            * images saved
        * a %-age of images seen / saved i.e. the hit-rate

* [x] apply simple checks
    * [x] some skeets show as having `(i) Adult Content` when viewed in bluesky. We should extract this from the metadata we see and filter out any skeets with this flag.
    * [x] some skeets are labelled as "The author of this post has requested their posts not be displayed on external sites.". We should also filter these out, as an indicator of dodginess/sensitivity

* [x] skin-based checks
    * we want to use presence and absence of skin as an inclusion filter and an exclusion filter:
        * [ ] inclusion: any identified face bounding boxes must include at least some %-age of skin (guess 70% to begin with)
            * this needs to be defined as a new `min_face_skin_pct` in `archetype.toml`
            * all example images currently labelled as matching an archetype should pass this check
        * [ ] exclusion: any skin areas outside of an identified face must be limited. Suggest the remaining area that's allowed to be covered is small e.g. only 10% of area outside of a face bounding box can be skin
            * this needs to be defined as a new `max_outside_face_skin_pct` in `archetype.toml` 
            * see the `examples/8978262e-3540-4593-bf8f-dfaf4de2b27f.png` image as one example which should be labelled as rejection for a reason of Rejection::TooMuchSkinOutsideFace
    * suggested implementation, in a new `skin-detection` crate:
        * [x] find an ML model we can use in Rust or an existing Rust library that categorises individual pixels as skin or not based on ranges of colors that are expected to come from skin; we should use existing science as much as possible and try to account for skin color of different ethnicities
        * [x] use this to take an image and produce a binary image which is the boolean yes/no for each pixel on whether it is skin
        * [x] update the annotations so that this is used as a mask on the original image
        * [x] apply the inclusions/exclusions logic by running skin-detection alongside face-detection and then combining the outputs together as needed

## Slice 4: False positive: Removing text

* [x] apply cosmetics
    * [x] update `skeet-finder` so that it keeps count of how often each rejection reason is seen, and shows that alongside a percentage

* the sizes of images are quite large, and filling up local disk.
    * For now, store on my NAS:
        * [x] update all references to `store` to instead go to `/Volumes/home/bobby/store `; do this in the `Justfile` by pulling out a STORE variable and using that
        * [x] to validate that saving and loading to a location can work let's add a small `validate-storage` cli:
            * [x] the `open` method of `SkeetStore` should also create a new `validate_v1` table (if not present) with a schema of:
                * timestamp
                * random number
            * [x] add a new `validate()` method on `SkeetStore` which validates by
                * appending a new (timestamp, random number) row to `validate_v1` table
                * does a query by the random number and checks the timestamp found is the one it just created
            * [x] this validate method should be called by a new `validate-storage` cli in `skeet-store` and also by the main.rs, first thing after it calls `open`
            * [x] add a new `validate-storage` rule to the `Justfile`
        * ok looks like it is failing with this error:
        ```
        Error: Lance(Lance { source: IO { source: Generic { store: "LocalFileSystem", source: UnableToRenameFile { source: Os { code: 45, kind: Uncategorized, message: "Operation not supported" } } }, location: Location { file: "/Users/mxm/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/lance-table-2.0.0/src/io/commit.rs", line: 1095, column: 50 } } })
        ```
            * [x] this is probably some weirdness with the way this directory is mounted on my Synology NAS as an SMB volume. Ultimately I want to store this on an S3-compatible system so I don't want to spend too much time on this. However, let's do some minimal investigation.
            * **Investigation result:** LanceDB uses atomic file renames (`rename()`) for commits. macOS SMB mounts don't support this (OS error 45 = `ENOTSUP`). This is a fundamental limitation of SMB, not a bug in our code.
            * **Solutions considered and excluded (for now):**
                * Use NFS instead of SMB — better rename support, but still filesystem-level workaround
                * Run MinIO on the Synology via Docker/Container Manager — provides a local S3-compatible endpoint at `http://<NAS-IP>:9000`
                * Swith LanceDB to S3 backend — supported via `lancedb = { features = ["aws"] }` and `lancedb::connect("s3://bucket/path").storage_options(...)`
            * **Decision:** park this for now. When we move to S3 storage (which is the long-term plan), this problem goes away. For now, use a local store path.
    * [x] actually just revert back to using a local store for now

* [x] we'd like to remove any images that contain text as these are very unlikely to be the archetype we want
    * see `examples/5c228b9a-8115-498b-854d-08f2fad5f7f1.png` and `examples/09a57394-66aa-4745-b16c-523c02e6bf0d.png` as examples we should reject as Rejection::TooMuchText
    * all existing examples which are currently matched should continue to match an archetype i.e. not be filtered out
    * add a parameter to `shared/archetype.toml` of `max_glyphs_allowed` and initially set it to `5`
    * suggested implementation, in a new `text-detection` crate:
        * [x] estimate glyph count:
            * option 1: use an ML model or an existing Rust crate which can run OCR against an image
                * this should ideally be one which supports multiple languages as we want to exclude non-english as well
            * option 2: alternatively, find a more general model that can detect/count glyphs in an image even if it can't parse them to text
        * [x] reject any images that have too many glyphs
    
* [x] apply refactorings
    * [x] some of the responsibilities of different modules aren't great e.g. `classify` in `face-detection` knows about things like word-count. We need to refactor towards this responsibility split:
        * crates like `text-detection`, `skin-detection`, `face-detection` should be leaf crates which know about their own detection area and not be aware of others
            * as an indicator of this, they should only depend on `shared` crate and no others crates in the repo
        * any pulling together of decisions around classification, which depend on these, should happen in `skeet-finder`. So, for example:
            * [ ] `classify` and all related work done should move to `classify_and_store`
            * [ ] `tests/examples.rs` in `face-detecion` should move to `skeet-finder`
            * [ ] `bin/classify_examples.rs` in `face-detecion` should move to `skeet-finder`
    * [x] actually, I don't think `classify_and_store` is needed anymore:
        * [x] `classify_image` can live in `classify.rs`
        * [x] `save` can be moved into the `main.rs` in `skeet-finder`
    * [x] generic things like `Rect` and `translate` should come from an external crate that has robust well-tested versions of these; we're not doing anything hugely complicated here with them, so probably correct, but to reduce the downstream burden we should research and find a replacement from a robust crate that is specialised for this and commonly used.

* [x] usability / debugging:
    * [x] indicate what the detected text was:
        * [x] ideally, update annotate to show the bounding box on image where text was detected
        * [x] add a new column in images table which is `detected_text` (UTF-8) (this changes the version)
        * [x] update `classify-examples` so that it also says what the detected text was
    * [x] add columns in `skeet-feed` UI:
        * [x] show the version of config that was used
        * [x] show the detected text

* [x] let's add metadata for the images that are exemplars i.e. really good examples of what we want, and then make changes as needed to ensure they are recognised
    * [x] add `exemplar` = True/False property to the items in examples/expected.toml with the following labelled as exemplar, and everything else not
        * examples/4472a427-f6bd-4e55-87bd-86f5f91e187e.png (recognised at bottom left)
        * examples/eno7kayhhljgvgwc7ttdoojx_3mfev3xjylk2w_0.png (recognised at top right)
        * examples/bafkreiaj24jwns3psvpdkv2437ldxhnrwa64ypyxnsz2awq6lwrf3eidna.png (recognised at top right)
        * examples/jbbneqrt2fxcij3kjwxdu54m_3mfev4a57a22u_0.png (note that it is ok that this one is currently not recognised)

* [x] sanity-check that adult-based filtering is still working
    * [x] add a debug-focussed cli called `metadata-dump` in `skeet-finder` that:
        1. can take an image id, find it in the store
        2. find the related skeet id
        3. make a call to bluesky API's to dump all metadata about the skeet, including all the attributes we are currently using for filtering but anything else available is good
    * [x] it looks like we are missing some coverage of tags that indicate adult content. We'll fix this in two ways:
        * [x] add an integ test which is driven by a blocklist of `at` URLs (libtest-mimic could be good for this) which should:
            1. read from a config of blocked URLs
            2. load the saved JSON for skeet
            3. verify it is blocked by the appropriate sub-part of the skeet-finder focussed on adult or similar content
        * [x] add a small Rust cli which can take an at URL, download the JSON, and add it and the URL to the blocklist
        * [x] do this for `at://did:plc:iyqgkr3bmlxwma7i63imodfs/app.bsky.feed.post/3mh4g7iacos23`
            1. add a covering test via tools above
            2. make changes to ensure we block anything that contains a `porn` label in it like this:
            ```
            "labels": [
                {
                "cid": "bafyreiaz5ut66dri5ijlk2acml7enpu5p3ezgdd3z3pr6vqgvdgfkzbppi",
                "cts": "2026-03-15T17:00:52.038Z",
                "src": "did:plc:ar7c4by46qjdydhdevvrndac",
                "uri": "at://did:plc:iyqgkr3bmlxwma7i63imodfs/app.bsky.feed.post/3mh4g7iacos23",
                "val": "porn",
                "ver": 1
                }
            ],
            ```
    * [x] `at://did:plc:3bebghh7fygmfwjo6udrye43/app.bsky.feed.post/3mh4gatfe2c2d` should be excluded because it is marked as `!no-unauthenticated"` (see JSON snippet below). Use `add_to_blocklist` for this and make the changes to ensure this is caught and filtered out.
    ```
    "labels": [
          {
            "cid": "bafyreia6fdcpkifef47kv7ad63rkgeoswaianmgmiw4g6erwrj2q3wfzbm",
            "cts": "1970-01-01T00:00:00.000Z",
            "src": "did:plc:3bebghh7fygmfwjo6udrye43",
            "uri": "at://did:plc:3bebghh7fygmfwjo6udrye43/app.bsky.actor.profile/self",
            "val": "!no-unauthenticated"
          }
        ]
    ```


## Slice 5: Make minimal version available online

What we want to get to is:
* `skeet-finder` still running locally on demand, but saving data to the cloud somewhere
* `skeet-feed` running as a website at `bobby.houseofmoran.io` which reads from the cloud

* [ ] update `skeet-finder` / `skeet-store` to save data to an S3-compatible location
    * I use https://bunny.net and would like to use their product for this (docs: https://docs.bunny.net/api-reference/storage / https://bunny.net/storage/)
    * I have already created a `hom-bobby` zone
    * desired usage:
        * for local usage, read-only and read-write keys are stored in 1Password and are accessed using dev integration
            * this can be done via passing a command-line argument to cli commands which is fetched in the Justfile via something like:
            ```
            READ_WRITE_KEY := `op read "op://Dev/hom-bobby-read-write/password"`
            ```
            * alternatively, if there is a native integration in Rust to fetch secrets from 1Password I'd prefer to use that and only pass the name of the secret (`Dev/hom-bobby-read-write`)
        * the URL for the S3 bucket should be passed into the CLI for `skeet-finder`
            * it should continue to also work for local dirs as well i.e. it's not forced to always use remote S3
    * [ ] also need to update `validate-storage` to work with this

* [ ] update `skeet-feed` to run on fly.io and read from the bunny S3 location
    * secrets will be managed using fly secrets which I will separately set up i.e. I will create `HOM_BOBBY_READ_ONLY` secret
    * desired usage:
        * right now there should be no reason for it to do any writes so only the read-only password will be given
        * the skeet-feed should be updated like the finder to read from the S3 bucket or a local dir
        * this will be deployed and published manually from my machine and there will be a set of integ tests which validate deployment was successful. there should also be a `bobby-prod` and `bobby-staging` version
            * see https://github.com/mikemoraned/fosdem/blob/main/Justfile and related setup for an example of how to do this

* [ ] add observability:
    * [ ] we should switch to tokio-tracing and associated observability setup support; keep it as simple as possible
        * the intent is to make this our pluggable layer which allows following to be done without lots of bespoke support or awareness of where info ends up
        * for example, any shared code used by `skeet-finder` and `skeet-feed` like `skeet-store` or `shared` should be unaware of deployment context
    * [ ] `skeet-finder` should be updated to, when running locally, output to a local `logs` dir, following timestamped files, like in unix, with auto-rollover etc
        * it should continue to have a persistent indicatif local UI
    * [ ] `skeet-feed` should log traces to a cloud-based opentracing provider
        * my preference is honeycomb, where I have an existing free account

## Slice N ...

# Target Architecture

Overall we want to get to a:
* skeet-finder
    * this continuously listens to the Bluesky firehose and detects skeets which contain images showing the content we want, then stores them in the skeet-store
* skeet-store
    * this stores the found skeets in an S3-compatible store, in tables, managed as [Lancedb](https://lancedb.com) tables
* skeet-feed
    * this an HTTP service which reads from the store and surfaces all skeets which have been found as a Bluesky Feed


