use shared::{Band, NormalizedScore, RefineModels, Score, Threshold};
use skeet_store::ModelVersion;
use tracing::warn;

/// Rescale a model's score so its `decision_threshold` maps to `0.5`, making
/// scores produced by models with different thresholds comparable.
///
/// Threshold-anchored piecewise-linear map `T_t`: `[0, t]` is stretched onto
/// `[0, 0.5]` and `[t, 1]` onto `[0.5, 1]`. It is monotone in `score`,
/// `>= 0.5` iff `score >= threshold` (i.e. iff [`shared::RefineModel::is_positive`]),
/// and the identity when `threshold == 0.5`. This is the feed/quality calibration
/// *policy*, so it lives here rather than on the shared type.
pub fn normalize(score: Score, threshold: Threshold) -> NormalizedScore {
    let x = f64::from(score);
    let t = f64::from(threshold);
    let n = if x <= t {
        if t == 0.0 { 0.5 } else { 0.5 * x / t }
    } else {
        // `x > t` implies `t < 1`, so the denominator is non-zero.
        (0.5f64.mul_add(x, 0.5) - t) / (1.0 - t)
    };
    // `T_t` maps `[0,1]×[0,1]` into `[0,1]` and is never NaN here (the `t == 0`
    // case is handled above); the clamp guards only `f32` rounding at the
    // endpoints, so `new` cannot fail.
    #[allow(clippy::expect_used)]
    NormalizedScore::new(n.clamp(0.0, 1.0) as f32)
        .expect("normalized score is in [0, 1] by construction")
}

/// Per-image effective band: a manual override wins; otherwise the score is
/// normalised by the producing model's threshold and banded.
///
/// An unknown `model_version` cannot be calibrated, so its band is floored to
/// `MediumLow` (below feed visibility)
pub fn image_effective_band(
    score: Score,
    model_version: &ModelVersion,
    models: &RefineModels,
    manual_image_band: Option<Band>,
) -> Band {
    if let Some(band) = manual_image_band {
        return band;
    }
    models.get(model_version).map_or_else(
        || {
            warn!(
                %model_version,
                "model_version not found in RefineModels — flooring effective band below feed visibility"
            );
            Band::MediumLow
        },
        |model| Band::from_normalized(normalize(score, model.decision_threshold)),
    )
}

/// The effective band for a whole skeet: the lowest contributing band.
///
/// The minimum of its manual override (if set) and every image's effective band.
/// `None` iff there are no images — the `min` over an empty set is undefined and
/// such a skeet is never visible. Sharing this between visibility and the quality
/// sort keeps the two from drifting apart.
pub fn skeet_effective_band(
    manual_skeet_band: Option<Band>,
    image_effective_bands: &[Band],
) -> Option<Band> {
    if image_effective_bands.is_empty() {
        return None;
    }
    manual_skeet_band
        .into_iter()
        .chain(image_effective_bands.iter().copied())
        .min()
}

/// Whether a skeet should appear in the feed: its [`skeet_effective_band`] is visible.
pub fn skeet_visible_in_feed(
    manual_skeet_band: Option<Band>,
    image_effective_bands: &[Band],
) -> bool {
    skeet_effective_band(manual_skeet_band, image_effective_bands)
        .is_some_and(Band::is_visible_in_feed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use shared::RefineModel;
    use shared::refine_model::{ModelName, ModelProvider, RefinePrompt};

    fn score(value: f32) -> Score {
        Score::new(value).expect("valid")
    }

    fn thr(value: f64) -> Threshold {
        Threshold::new(value).expect("valid")
    }

    fn models_with(version: &str, threshold: f64) -> RefineModels {
        let mut models = RefineModels::new();
        models.insert_unverified(
            version,
            RefineModel {
                model_provider: ModelProvider::openai(),
                model_name: ModelName::gpt_4o(),
                prompt: RefinePrompt::new("test"),
                decision_threshold: thr(threshold),
            },
        );
        models
    }

    fn any_band() -> impl Strategy<Value = Band> {
        proptest::sample::select(Band::ALL)
    }

    /// `normalize` is the identity at `t = 0.5` and stretches each half
    /// proportionally for other thresholds (threshold → 0.5).
    #[test]
    fn normalize_examples() {
        let approx = |a: f32, b: f32| (a - b).abs() < 1e-6;

        // identity at t = 0.5
        for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let n: f32 = normalize(score(v), thr(0.5)).into();
            assert!(approx(n, v), "normalize({v}, 0.5) = {n}, expected {v}");
        }

        // t = 0.6: threshold → 0.5, lower half compressed, upper half stretched.
        let at: f32 = normalize(score(0.6), thr(0.6)).into();
        assert!(approx(at, 0.5));
        let below: f32 = normalize(score(0.3), thr(0.6)).into(); // 0.5 * 0.3 / 0.6
        assert!(approx(below, 0.25));
        let above: f32 = normalize(score(0.8), thr(0.6)).into(); // (0.4 + 0.5 - 0.6) / 0.4
        assert!(approx(above, 0.75));
    }

    proptest! {
        /// `normalize` is monotone in score for a fixed threshold.
        #[test]
        fn normalize_monotone(
            a in 0.0f32..=1.0f32,
            b in 0.0f32..=1.0f32,
            t in 0.01f64..=1.0f64,
        ) {
            if a <= b {
                prop_assert!(normalize(score(a), thr(t)) <= normalize(score(b), thr(t)));
            }
        }

        /// `normalize(s, t) >= 0.5` iff `s >= t` — the positivity/visibility boundary.
        /// A `>= 0.02` gap each side of the threshold keeps `f32` rounding from
        /// flipping the boundary (the exact-boundary case is covered concretely).
        #[test]
        fn normalize_half_iff_above_threshold(t in 0.05f64..=0.95f64, delta in 0.02f32..=0.4f32) {
            let t32 = t as f32;
            let below = (t32 - delta).max(0.0);
            let above = (t32 + delta).min(1.0);
            let below_n: f32 = normalize(score(below), thr(t)).into();
            let above_n: f32 = normalize(score(above), thr(t)).into();
            prop_assert!(below_n < 0.5, "normalize({below}, {t}) = {below_n} should be < 0.5");
            prop_assert!(above_n >= 0.5, "normalize({above}, {t}) = {above_n} should be >= 0.5");
        }

        /// A manual image band always wins over the model-derived band.
        #[test]
        fn image_manual_overrides_score(
            score_v in 0.0f32..=1.0f32,
            band in any_band(),
            t in 0.05f64..=0.95f64,
        ) {
            let models = models_with("m", t);
            let mv = ModelVersion::from("m");
            prop_assert_eq!(
                image_effective_band(score(score_v), &mv, &models, Some(band)),
                band,
            );
        }

        /// Without a manual override, the effective band is the model-normalised band.
        #[test]
        fn image_no_manual_uses_model_band(score_v in 0.0f32..=1.0f32, t in 0.05f64..=0.95f64) {
            let models = models_with("m", t);
            let mv = ModelVersion::from("m");
            prop_assert_eq!(
                image_effective_band(score(score_v), &mv, &models, None),
                Band::from_normalized(normalize(score(score_v), thr(t))),
            );
        }
    }

    /// A score from a model not in the registry is floored below feed visibility,
    /// regardless of how high the raw value is.
    #[test]
    fn unknown_model_floors_below_feed() {
        let models = models_with("known", 0.5);
        let band = image_effective_band(score(0.99), &ModelVersion::from("stale"), &models, None);
        assert_eq!(band, Band::MediumLow);
        assert!(!band.is_visible_in_feed());
    }

    #[test]
    fn skeet_effective_band_is_the_min() {
        assert_eq!(
            skeet_effective_band(None, &[Band::HighQuality, Band::MediumHigh]),
            Some(Band::MediumHigh),
        );
        assert_eq!(
            skeet_effective_band(Some(Band::Low), &[Band::HighQuality]),
            Some(Band::Low),
        );
        assert_eq!(skeet_effective_band(None, &[]), None);
    }

    #[test]
    fn no_images_means_not_visible() {
        assert!(!skeet_visible_in_feed(None, &[]));
        assert!(!skeet_visible_in_feed(Some(Band::HighQuality), &[]));
    }

    proptest! {
        /// Visibility iff every contributing band (manual skeet + all images) is visible.
        #[test]
        fn skeet_visible_iff_all_bands_visible(
            manual_skeet in proptest::option::of(any_band()),
            images in proptest::collection::vec(any_band(), 1..=5),
        ) {
            let all_visible = manual_skeet.map_or(true, |b| b.is_visible_in_feed())
                && images.iter().all(|b| b.is_visible_in_feed());
            prop_assert_eq!(skeet_visible_in_feed(manual_skeet, &images), all_visible);
        }

        /// A non-visible manual skeet band always hides the skeet regardless of images.
        #[test]
        fn manual_demote_always_hides(
            band in any_band(),
            images in proptest::collection::vec(any_band(), 1..=5),
        ) {
            prop_assume!(!band.is_visible_in_feed());
            prop_assert!(!skeet_visible_in_feed(Some(band), &images));
        }

        /// Any non-visible image band taints the whole skeet.
        #[test]
        fn one_bad_image_taints_skeet(
            manual_skeet in proptest::option::of(any_band()),
            good in proptest::collection::vec(any_band(), 0..=4),
            bad in any_band(),
        ) {
            prop_assume!(!bad.is_visible_in_feed());
            let mut images = good;
            images.push(bad);
            prop_assert!(!skeet_visible_in_feed(manual_skeet, &images));
        }
    }
}
