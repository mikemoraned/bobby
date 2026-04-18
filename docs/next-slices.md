# Next Slices

## Slice 15: re-introduce text-filtering to reduce costs / increase quality

### Target

As of 15th April, I am seeing a lot of low-quality skeets come through with text in them. We previously were applying text filtering but it didn't seem to be of much value, as it was only excluding a small %-age. It may be that were just lucky before and now it'd be more useful.

Now that we have an ability to manually appraised skeet images by quality we should use this to establish a test set.

So, what we want is:
* a manually-appraised set (200 should be enough) of images
* text-based pruning re-applied, perhaps differently to before
* a measurement of precision on this test set before/after pruning by text

### Tasks

#### Cleanups

* [ ] I added `tokio-console` support but I've not really used it. I originally added it because I thought it was more like a local telemetry viewer than async debugger. So, generally useful, but not for most of what I've been doing. TLDR: support for it should be removed, and dependency can be deleted.

#### Bugs

* [ ] is auth login actually working for github admin when deployed?

#### Manual Appraisal

* [ ] extend feed admin pages to show overall counts of number of appraised skeets and images, on respective views
* [ ] appraise 200 images

#### (Imperfect) Precision/Recall measure

* [ ] write a small CLI in `skeet-prune` called `eval` which:
    1. finds all images in a store that have been manually appraised into a particular Band i.e. ignore anything not manually appraised
        * this may involve adding support to `SkeetStore` for this
    2. map the Band for an image to a binary `should_be_pruned` variable:
        * Band = Low, then should be pruned, `should_be_pruned` = true
        * Band = anything else, then may be allowed, `should_be_pruned` = false
    3. fetch the information for these images we want to assess, and for each, run them through a classify pass, where we collect whether an image would have been pruned or not
    4. do precision/recall evaluation by taking `should_be_pruned` as the actual, and whether it was pruned in step 3 as the prediction
        * note that as overall measures these are skewed, as the only images that have been appraised are the ones that previously had not been pruned. so we are biasing towards only examining that subset, and not the wider unknown set that was never seen by a person. this is ok, as we are using this here as a way to see if text-detection can be a narrower more precise way to exclude images. We are aiming to measure an increase in precision and no loss of recall, and this is measurement method is sufficient for that.

#### Re-introduce text-based filtering as an optional filter

* [ ] go back through commit history and bring back the text-detection crate contents. don't yet hook it into any classification i.e. we won't use it for real yet
* [ ] run mutation-testing on this, to flush out any testing gaps. also migrate any tests to prop-based style
* [ ] make classification methods configurable by making it so that we can optionally use text-detection, but face-detection and skin-detection are on by default.

#### Evaluate text-detection

* [ ] using above capabilities, do two runs of `eval` one with defaults (no text detection) and one with text-detection enabled and compare performance
    * it may be overkill, but `eval` could be extended to do the shared steps (1+2) and then run two different classification configs side-by-side on the same data; this way we ensure we are comparing like-for-like

## Slice 16: reducing unintentional bias

### Target

The current skin-detection method in `lib.rs` (Kovac/Peer/Solina 2003 RGB rules + a YCbCr box) is biased toward lighter skin tones. Replace it with a method that performs more fairly across the Fitzpatrick scale, and add tests that would catch this kind of regression in future.

### Tasks

#### Document and demonstrate the current bias
- [ ] Write up the specific lines in `is_skin_pixel` that exclude darker skin:
    - [ ] `r <= 95.0` reject — eliminates much dark brown skin outright
    - [ ] `g <= 40.0` / `b <= 20.0` rejects — fail in shadow and on very dark skin
    - [ ] `(r - g).abs() <= 15.0` reject — absolute R−G gap shrinks at lower intensities even when the ratio is preserved
    - [ ] `max - min <= 15.0` reject — same low-intensity compression problem
    - [ ] note that the YCbCr box is the least-biased part but is ANDed with the RGB gate, so the RGB rules dominate failures
- [ ] Add failing unit tests with known dark-skin RGB samples (e.g. `(80, 50, 35)`, `(60, 40, 30)`, `(110, 75, 55)`) asserting they should be classified as skin — these should fail against the current implementation and pass against the replacement
- [ ] Assemble a small evaluation set of face images spanning Fitzpatrick I–VI and measure per-bucket true-positive rate before and after the change

#### Pick a less-biased method
- [ ] Evaluate options in roughly increasing order of effort:
    - [ ] **CbCr-only elliptical region** (Hsu, Abdel-Mottaleb & Jain, 2002) — drop the RGB gate entirely, fit an ellipse in CbCr space rather than an axis-aligned box. Small code change, big fairness improvement.
    - [ ] **HSV or normalised-rgb thresholds** — hue ≈ [0°, 50°] with moderate saturation and *any* value; removes the luminance dependency that hurts dark skin
    - [ ] **Jones & Rehg statistical skin model** (1999) — Bayesian histogram trained on a large diverse pixel set, runtime is a 3D lookup table, still the standard classical baseline
    - [ ] **Modern ML model trained on a diverse dataset** — anything evaluated on Fitzpatrick 17k or trained on FSD/ECU/Pratheepan; highest accuracy, adds a dependency

#### Rust ecosystem options
- [ ] **Pure-Rust / classical (no new heavy deps).** There is no dedicated "less-biased skin detector" crate on crates.io — closest neighbours are face-detection crates, not skin segmentation. So this path is hand-rolled on top of the existing `image` crate:
    - [ ] Implement a CbCr-ellipse or HSV test directly in `lib.rs`
    - [ ] Optionally fit a Jones-and-Rehg histogram offline against the [UCI Skin Segmentation dataset](https://archive.ics.uci.edu/ml/datasets/skin+segmentation) (built from face images "of diversity of age, gender, and race") and ship the resulting lookup table as a `.bin` in the repo
- [ ] **ML model via ONNX.** The standard route for running pretrained vision models from Rust is the [`ort`](https://crates.io/crates/ort) crate (ONNX Runtime bindings). [`rust-faces`](https://crates.io/crates/rust-faces) is a good template for how to wire an ONNX model into a Rust API similar to our `detect_skin` signature.
- [ ] **Candidate model:** [samhaswon/skin_segmentation](https://github.com/samhaswon/skin_segmentation) on GitHub — a benchmark/training repo with ONNX exports of several skin-segmentation models (BiRefNet, U²-Net variants, etc.). The author explicitly built the training set "to maximize diversity of scene, lighting, and skin appearance" with augmentations designed so the model isn't dependent on lighting or camera settings. Caveat: the heaviest BiRefNet variant uses ~40 GB RAM through onnxruntime, so pick one of the smaller CNN models.
- [ ] **Background reading** for evaluation methodology and fairness framing:
    - [ ] [Fitzpatrick 17k](https://github.com/mattgroh/fitzpatrick17k) (Groh et al., 2021) — standard fairness benchmark
    - [ ] [Bencevic et al. (2024)](https://www.sciencedirect.com/science/article/pii/S0169260724000403) — quantifies the same bias pattern across U-Net-based skin segmentation models

#### Recommended path
- [ ] **Step 1 — cheap win.** Replace the RGB+CbCr-box rules with either a CbCr ellipse or a Jones-and-Rehg histogram trained on the UCI dataset. Pure Rust, no new heavy deps, almost certainly closes most of the gap. Add the dark-skin unit tests above so the improvement is visible.
- [ ] **Step 2 — only if step 1 isn't good enough.** Add `ort` and load one of the smaller models from samhaswon/skin_segmentation behind an optional feature flag (`features = ["ml"]`), keeping the classical path as the default so the binary stays small.

#### Guardrails
- [ ] Keep the per-Fitzpatrick-bucket eval as a checked-in test or bench so future changes can't silently regress fairness
- [ ] Update the doc-comment on `detect_skin` to honestly describe what the method does and its known limitations
