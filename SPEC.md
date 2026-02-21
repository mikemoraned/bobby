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
    * we should write out the scores and the identifiers as a table to a directory in parquet format. This table should contain an identifier that allows us to get back to the original bluesky message.

We'll implement this in steps which we will tick-off as we go and/or change based on what we discover:
* [ ] listen to the bluesky firehose
    * it's enough to just be writing-out a log message of what is received
* [ ] find any messages that contain images
* [ ] ... more todo's added here as we need them

# Constraints, trade-offs and technology choices

* Where possible, all code should be written in Rust.
    * It's acceptable to use a non-Rust language or toolset for the purpose of getting the bluesky firehose data, including images. However, once an image is fetched everything else must be in Rust.
    * Note that it's ok to use non-Rust ML models.
* Use existing models, or Rust libraries, for face-detection and landmark identification.
* Please use Burn (https://burn.dev / https://github.com/tracel-ai/burn / https://burn.dev/get-started/) for running ML Models
* It may not be possible to process images at the same rate as they are received (line-speed). This is totally fine. In this case we can adopt a sampling approach where we sample from the stream at the rate at which we can process images.
    * However, we should aim to do the really simple parts inline with receiving a message. For example, identifying if a message contains an image is likely something we can do at line-speed.