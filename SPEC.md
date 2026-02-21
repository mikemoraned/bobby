# Context

This is being done in the context of a "makers gonna make" session where the focus is on doing something end-to-end in one day (at least that's my focus).

# Purpose

Find selfies people take of themselves with physical landmarks (famous buildings, monuments, places like the Eiffel Tower).

I've already done this once before, but with Twitter: https://www.houseofmoran.com/post/126043044893/looking-for-bobby-but-found-paris-instead/. The purpose is to recreate this but using bluesky instead of twitter, and using modern technologies. See [original repo](https://github.com/mikemoraned/selfies) for context.

The original project scanned Twitter's firehose, applied face detection (via OpenIMAJ), and looked for selfie-like compositions (face in foreground border, landmark in background). It had a ~0.1% hit rate and encountered challenges like porn, screenshots, and false positives from inanimate objects resembling faces.

# Todo

The process should be similar to the original i.e.
1. Listen to the Bluesky firehose via the Jetstream WebSocket API (`wss://jetstream2.us-east.bsky.network/subscribe`), filtering for `app.bsky.feed.post` messages
2. Apply hard-constraint filtering steps i.e. every image should have this:
    * message should contain an attached image
3. Apply scoring. These are all attributes that should increase the likelihood of being regarded as a selfie:
    * the face should sit in the border of the image i.e. if you were to split an image into a 9x9 grid, the face should not be in the central grid entry.
    * the image should contain a landmark of some kind
4. Dump candidates into a folder:
    * each image candidate should be stored in a folder called "candidates" and have a unique id as part of the name
        * each image should have a companion image which is the original but with bounding-boxes around the face and landmark (if present)
    * we should write out the scores and the identifiers as a table in a sqlite db. This table should contain an identifier that allows us to get back to the original bluesky message.

We'll implement this in steps which we will tick-off as we go and/or change based on what we discover:
* [x] listen to the bluesky firehose
    * [x] it's enough to just be writing-out a log message of what is received
    * [x] apply an inline test that captures what we've done (also do this for subsequent steps)
* [x] find any messages that contain images
* [x] find any images that contain faces
    * [x] first, just log out something if we find a face i.e. don't write anything out to disk
    * [x] then, write out the image file and annotated file to `candidates` dir as a jpeg. So, for example, if image or message had `id`, then we'd create files:
        * `candidates/id.png`
        * `candidates/id_annotated.png`
* [ ] filter by landmark
    * [ ] find any images that also contain a landmark
    * [ ] filter to only save images that contain a landmark and a face
    * [ ] add the landmark bounding box to annotated images, but in a different color to faces
* [ ] score based on structure of image
    * [ ] assign a score based on aspects of the detected bounding boxes (bb) positions, and certainty of match
        * an initial version of this can be scored based on different aspects of placement e.g.
            * where is the face bb positioned? if in middle (center of 9x9 grid) then score is 0.0, but if outside that, then score is 1.0
            * what is extent of overlap between landmark bb and face bb? the score should be lower if overlap is higher i.e. we'd like to favour examples where the face and landmark don't overlap
            * multiply these through by certainty from each detector e.g. we end up a score with like:
                * score(image) = avg(landmark_certainty, face_certainty) * face_position_score * overlap_score
    * [ ] output the score into a table containing:
        * identifier of image
        * timestamp of when discovered (this is the local processing time when we first saw it)
        * timestamp of original (this is when the message was posted)
        * local paths of saved image and annotated image
        * overall score
        * score of components
* [ ] ... more todo's added here as we need them

# Constraints, trade-offs and technology choices

* Where possible, all code should be written in Rust.
    * It's acceptable to use a non-Rust language or toolset for the purpose of getting the bluesky firehose data, including images. However, once an image is fetched everything else must be in Rust.
    * Note that it's ok to use non-Rust ML models.
* Use existing models, or Rust libraries, for face-detection and landmark identification.
* Please use Burn (https://burn.dev / https://github.com/tracel-ai/burn / https://burn.dev/get-started/) for running ML Models
* It may not be possible to process images at the same rate as they are received (line-speed). This is totally fine. In this case we can adopt a sampling approach where we sample from the stream at the rate at which we can process images.
    * However, we should aim to do the really simple parts inline with receiving a message. For example, identifying if a message contains an image is likely something we can do at line-speed.
* any command-line invocations should be captured in the Justfile

## Invariants / Style

* models:
    * any models should be downloaded to `models` dir
* commenting / docs:
    * focus should be on making the code itself self-documenting as opposed to requiring extra commenting
    * comments can still be added, but should be focussed on not something covered by the code. However, even then, it's probably better to create a new section in SPEC.md or create a separate markdown doc if needed.
* stability / quality. Where possible we should follows these protections:
    * don't use `-pre` versions of dependencies
    * don't use direct git versions of dependencies
* rust-specifics:
    * see above general guidance about comments. However, if comments are needed, please use inline Rust conventions for function comments.
    * always use latest rust version and edition where possible, but do not use rust nightly
        * specify the rust version in `rust-toolchain.toml` and the edition in `edition` in `Cargo.toml`
    * always apply `cargo clippy` after completion of each todo we complete
    * where possible we should:
        * follow the [NewType](https://doc.rust-lang.org/rust-by-example/generics/new_types.html) idiom e.g. we should avoid having any bare Strings.
        * use types rather than untyped arrays. 
            * For example, when passing images, use things like `DynamicImage` or similar, instead of using an array of byes.
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