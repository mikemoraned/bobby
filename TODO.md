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
* use external crates for core things like datetimes etc
* always use latest rust version and edition where possible, but do not use rust nightly
    * specify the rust version in `rust-toolchain.toml` and the edition in `edition` in `Cargo.toml`
* usage of `unwrap`:
    * this is denied by default. if absolutely needed, please annotate with `#[allow(clippy::unwrap_used)]` and give a justification
* always apply `just clippy` after completion of each todo we complete
* where possible we should:
    * follow the [NewType](https://doc.rust-lang.org/rust-by-example/generics/new_types.html) idiom e.g. we should avoid having any bare Strings.
    * use types rather than untyped arrays. 
        * For example, when passing images, use things like `DynamicImage` or similar, instead of using an array of bytes.
    * where there is a possibility of something being missing, we should capture that as an Option::None, or a Result::Err
        * use Option::None when the item missing leaves the overall sub-system valid i.e. if it is expected or allowed for this to happen
        * use Result::Err when it represents an invalid state. In this situation the caller should call the method with `?` and consider if the error is significant enough that the program should stop.
* error representations:
    * errors should use structured Enums to represent the different causes of the error. Use [thiserror](https://docs.rs/thiserror/latest/thiserror/) for this.
* logical structuring:
    * roughly-speaking, anything that is a different kind of thing (e.g. a schema for a message) or a different layer (e.g. core message routing or image labelling) should live in it's own module, and have it's own tests.
* testing:
    * for each core piece of functionality, it should have an associated inline unit test
    * for anything that requires multiple parts to prove it works, we should have an integ test
        * for this, we likely want to capture a real (but small) dump of firehose data to use
    * if possible our tests should be high-level and assert invariants rather than bespoke individual examples. in other words, we should use tests that have a high leverage between number of lines of test and breadth of behaviour tested
        * something like https://docs.rs/quickcheck/latest/quickcheck/ may be useful here

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

* [ ] create a skeet-feed which:
    * [ ] find all unique skeets from the `images_v1` table
    * [ ] surfaces these just as a web-page which shows embedded skeets i.e. no actual Bluesky feed needed yet

## Slice 2: finding faces

We're now going to start using some real models to find and detect faces.

* [ ] update skeet-finder so that, instead of randomly selecting one image, it:
    * [ ] only allows through images which contain at least one face. This face must be detected as being face-on i.e. side-profile faces are not allowed.
        * the "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx" ONNX model is one we've previously used which may be suitable here
        * document any model choices in `doc` dir
    * [ ] matches to an archetype where the face is of a single person, and that persons face sits in one quadrant of the image.
        * this matching should be captured in a Archetype enum, which should be saved as an extra column of the images table
            * this is a backwards incompatible change, so table should now be name `images_v2`
        * see examples dir for example images which you should capture in a test:
            * examples/eno7kayhhljgvgwc7ttdoojx_3mfev3xjylk2w_0.png : Archetype::TOP_RIGHT
            * examples/jbbneqrt2fxcij3kjwxdu54m_3mfev4a57a22u_0.png : Archetype::BOTTOM_LEFT

## Slice N ...

# Target Architecture

Overall we want to get to a:
* skeet-finder
    * this continuously listens to the Bluesky firehose and detects skeets which contain skeets with images showing the content we want, then stores them in the skeet-store
* skeet-store
    * this stores the found skeets in an S3-compatible store, in tables, managed as [Lancedb](https://lancedb.com) tables
* skeet-feed
    * this an HTTP service which reads from the store and surfaces all skeets which have been found as a Bluesky Feed


