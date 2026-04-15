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

    fn score(v: f32) -> Score {
        Score::new(v).expect("valid score")
    }

    #[test]
    fn image_effective_no_manual() {
        assert_eq!(image_effective_band(score(0.1), None), Band::Low);
        assert_eq!(image_effective_band(score(0.8), None), Band::HighQuality);
    }

    #[test]
    fn image_effective_manual_overrides() {
        assert_eq!(
            image_effective_band(score(0.8), Some(Band::Low)),
            Band::Low
        );
        assert_eq!(
            image_effective_band(score(0.1), Some(Band::HighQuality)),
            Band::HighQuality
        );
    }

    #[test]
    fn no_images_means_not_visible() {
        assert!(!skeet_visible_in_feed(None, &[]));
        assert!(!skeet_visible_in_feed(Some(Band::HighQuality), &[]));
    }

    #[test]
    fn all_high_quality_is_visible() {
        let bands = vec![Band::MediumHigh, Band::HighQuality];
        assert!(skeet_visible_in_feed(None, &bands));
    }

    #[test]
    fn one_bad_image_taints_skeet() {
        let bands = vec![Band::HighQuality, Band::Low];
        assert!(!skeet_visible_in_feed(None, &bands));
    }

    #[test]
    fn manual_skeet_override_cannot_rescue_bad_images() {
        let bands = vec![Band::Low, Band::MediumLow];
        assert!(!skeet_visible_in_feed(Some(Band::HighQuality), &bands));
    }

    #[test]
    fn manual_skeet_demote_hides_good_images() {
        let bands = vec![Band::HighQuality, Band::MediumHigh];
        assert!(!skeet_visible_in_feed(Some(Band::Low), &bands));
    }

    #[test]
    fn visible_when_skeet_and_images_all_good() {
        let bands = vec![Band::MediumHigh];
        assert!(skeet_visible_in_feed(Some(Band::HighQuality), &bands));
    }
}
