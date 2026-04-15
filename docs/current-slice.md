# Current Slice: Slice 14 — Property-based tests for value types

### Target

Adopt [`proptest`](https://docs.rs/proptest/latest/proptest/) for value-type tests across the workspace. The codebase is currently example-based throughout, but several value types are textbook property-test candidates: validity ranges, parse/display roundtrips, and ordering invariants. Convert the strongest candidates and use them as the template for future tests.

### Tasks

#### Set up
- [ ] Add `proptest` to `[workspace.dependencies]` and as a `dev-dependency` of `shared`, `skeet-store`, and `skeet-web-shared`.

#### Convert strongest candidates first
- [ ] **`Score`** (`shared/src/score.rs`) — collapse the 6 example tests into properties:
    - validity: `∀ f32 x: Score::new(x).is_ok() ⟺ 0.0 ≤ x ≤ 1.0`
    - parse/display roundtrip: `∀ valid Score s: s.to_string().parse() == Ok(s)` (mod float precision)
    - ordering matches the underlying f32 ordering
- [ ] **`Percentage`** (`shared/src/lib.rs`) — validity + ordering properties. Note: `Percentage::new` currently panics on invalid input; refactor to return `Result` first.
- [ ] **`ImageId` V1 and V2** (`skeet-store/src/types.rs`) — parse/display roundtrip; "different content yields different V2 id" over arbitrary byte slices instead of two hardcoded image sizes.
- [ ] **`SkeetId`** (`shared/src/skeet_id.rs`) — parse/display roundtrip over arbitrary valid `(did, collection, rkey)` triples; rejection of arbitrary malformed strings.
- [ ] **`Band`** (`shared/src/band.rs`) — `from_score` totality, monotonicity, and visibility-threshold equivalence; parse/display roundtrip.

#### Plug existing gaps
- [ ] **`Rejection`** roundtrip test (`shared/src/rejection.rs`) currently only covers 2 of 8 variants. Replace with an exhaustive iteration (or a property over an `Arbitrary<Rejection>`) so adding a new variant without a matching `FromStr` arm fails the test.
- [ ] **`Zone`** (`shared/src/zone.rs`) — parse/display roundtrip over all 9 variants (currently 1 test that iterates manually).
- [ ] **`Appraiser`** (`shared/src/appraiser.rs`) — parse/display roundtrip for valid appraisers; rejection of empty identifiers and unknown providers over arbitrary strings.

#### Lower-priority candidates
- [ ] **`PruneConfig::version()`** — property: equal configs hash equal; differing configs hash differently (with overwhelming probability).
- [ ] **`DiscoveredAt::is_within_hours`** — time-arithmetic invariants over arbitrary timestamps and hour windows.
- [ ] **Effective band logic** (`skeet-web-shared/src/effective_band.rs`) — properties for manual-override semantics: manual demote always hides; manual promote at skeet level always wins over automatic; "one bad image taints the whole skeet" holds across all (manual, automatic) combinations.

#### Guardrails
- [ ] Keep the example tests as named regressions where they encode a specific historical bug or boundary case worth documenting; otherwise remove them when the property-based version subsumes them (per the "remove dead code" rule).
- [ ] Make sure properties run under `just test` with a sensible iteration count (default is usually fine).
