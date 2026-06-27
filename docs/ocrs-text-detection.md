# ocrs Text Detection & Recognition Models

## Source

- **Project**: [ocrs](https://github.com/robertknight/ocrs) — an OCR engine built on
  [rten](https://github.com/robertknight/rten), by Robert Knight.
- **Files**: `text-detection.rten` (detection) and `text-recognition.rten` (recognition).
- **Download**: `https://ocrs-models.s3-accelerate.amazonaws.com/{text-detection,text-recognition}.rten`
- **Format**: rten's native `.rten` format, loaded by the `ocrs` crate's `OcrEngine`.

## What they do

`ocrs` runs a two-stage OCR pipeline: the **detection** model finds regions of
text in an image, and the **recognition** model reads the characters within each
region. The `text-detection` crate wraps this as `TextDetector`, returning the
detected text regions (bounding box + recognized content) for an image.

## Why we use it

Bobby looks for selfies people take with physical landmarks. Text-heavy images —
screenshots, memes, infographics, advertising — are almost never that, so the
image stage uses the amount of detected text as a rejection signal
(`Rejection::TooMuchText`). Detecting *and* recognizing text (rather than just
detecting text-like regions) lets the prune config reason about how much real
text an image carries, not merely whether some text-shaped pixels exist.

