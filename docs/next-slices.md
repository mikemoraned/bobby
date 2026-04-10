# Next Slices

## Slice 14: reducing unintentional bias

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

## Slice 15: Property-based tests for value types

### Target

Adopt [`proptest`](https://docs.rs/proptest/latest/proptest/) for value-type tests in `shared` and `skeet-store`. The codebase is currently example-based throughout, but several value types are textbook property-test candidates: validity ranges, parse/display roundtrips, and ordering invariants. Convert the strongest candidates and use them as the template for future tests.

### Tasks

#### Set up
- [ ] Add `proptest` to `[workspace.dependencies]` and as a `dev-dependency` of `shared` and `skeet-store`.

#### Convert strongest candidates first
- [ ] **`Score`** (`shared/src/lib.rs`) — collapse the 6 example tests into properties:
    - validity: `∀ f32 x: Score::new(x).is_ok() ⟺ 0.0 ≤ x ≤ 1.0`
    - parse/display roundtrip: `∀ valid Score s: s.to_string().parse() == Ok(s)` (mod float precision)
    - ordering matches the underlying f32 ordering
- [ ] **`Percentage`** (`shared/src/lib.rs`) — validity + ordering properties.
- [ ] **`ImageId` V1 and V2** (`skeet-store/src/types.rs`) — parse/display roundtrip; "different content yields different V2 id" over arbitrary byte slices instead of two hardcoded image sizes.
- [ ] **`SkeetId`** (`shared/src/skeet_id.rs`) — parse/display roundtrip over arbitrary valid `(did, collection, rkey)` triples; rejection of arbitrary malformed strings.
- [ ] **`Band`** (added in slice 13) — `from_score` totality, monotonicity, and visibility-threshold equivalence; parse/display roundtrip.

#### Plug existing gaps
- [ ] **`Rejection`** roundtrip test (`shared/src/lib.rs:343`) currently only covers 2 of 8 variants. Replace with an exhaustive iteration (or a property over an `Arbitrary<Rejection>`) so adding a new variant without a matching `FromStr` arm fails the test.

#### Lower-priority candidates
- [ ] **`PruneConfig::version()`** — property: equal configs hash equal; differing configs hash differently (with overwhelming probability).
- [ ] **`DiscoveredAt::is_within_hours`** — time-arithmetic invariants over arbitrary timestamps and hour windows.
- [ ] **Effective band logic** (added in slice 13) — once it lands, add properties for manual-override semantics: manual demote always hides; manual promote at skeet level always wins over automatic; "one bad image taints the whole skeet" holds across all (manual, automatic) combinations.

#### Guardrails
- [ ] Keep the example tests as named regressions where they encode a specific historical bug or boundary case worth documenting; otherwise remove them when the property-based version subsumes them (per the "remove dead code" rule).
- [ ] Make sure properties run under `just test` with a sensible iteration count (default is usually fine).