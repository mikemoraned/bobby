use shared::{Band, Score};

/// Per-image effective band: manual override wins, otherwise derived from score.
pub fn image_effective_band(score: Score, manual_image_band: Option<Band>) -> Band {
    manual_image_band.unwrap_or_else(|| Band::from_score(score))
}

/// Whether a skeet should appear in the feed.
///
/// Takes the effective skeet band (manual override if set, otherwise absent)
/// and every image's effective band. Visibility is determined by the lowest
/// band across all of them — if any single band is not visible, the skeet
/// is hidden.
pub fn skeet_visible_in_feed(
    manual_skeet_band: Option<Band>,
    image_effective_bands: &[Band],
) -> bool {
    if image_effective_bands.is_empty() {
        return false;
    }

    let lowest = manual_skeet_band
        .into_iter()
        .chain(image_effective_bands.iter().copied())
        .min();

    lowest.is_some_and(|b| b.is_visible_in_feed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn any_band() -> impl Strategy<Value = Band> {
        proptest::sample::select(Band::ALL)
    }

    #[test]
    fn no_images_means_not_visible() {
        assert!(!skeet_visible_in_feed(None, &[]));
        assert!(!skeet_visible_in_feed(Some(Band::HighQuality), &[]));
    }

    proptest! {
        /// Manual image band always wins over the score-derived band.
        #[test]
        fn image_manual_overrides_score(score_v in 0.0f32..=1.0f32, band in any_band()) {
            let score = Score::new(score_v).expect("valid");
            prop_assert_eq!(image_effective_band(score, Some(band)), band);
        }

        /// Without a manual override, the effective band is derived from the score.
        #[test]
        fn image_no_manual_uses_score(score_v in 0.0f32..=1.0f32) {
            let score = Score::new(score_v).expect("valid");
            prop_assert_eq!(
                image_effective_band(score, None),
                Band::from_score(score),
            );
        }

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
