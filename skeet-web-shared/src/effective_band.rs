use shared::{Band, Score};

/// Per-image effective band: manual override wins, otherwise derived from score.
pub fn image_effective_band(score: Score, manual_image_band: Option<Band>) -> Band {
    manual_image_band.unwrap_or_else(|| Band::from_score(score))
}

/// Per-skeet auto band: the worst effective band across all of the skeet's images.
///
/// Returns `None` if `image_bands` is empty.
pub fn skeet_auto_band(image_bands: &[Band]) -> Option<Band> {
    image_bands.iter().copied().min()
}

/// Per-skeet effective band: manual skeet override wins, otherwise the auto band.
pub fn skeet_effective_band(manual_skeet_band: Option<Band>, auto_band: Band) -> Band {
    manual_skeet_band.unwrap_or(auto_band)
}

/// Whether a skeet should appear in the feed.
///
/// Both the effective skeet band and every individual image effective band must
/// be visible for the skeet to show.
pub fn skeet_visible_in_feed(effective_skeet_band: Band, image_effective_bands: &[Band]) -> bool {
    effective_skeet_band.is_visible_in_feed()
        && image_effective_bands
            .iter()
            .all(|b| b.is_visible_in_feed())
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
    fn skeet_auto_band_uses_worst() {
        assert_eq!(
            skeet_auto_band(&[Band::HighQuality, Band::MediumHigh]),
            Some(Band::MediumHigh)
        );
        assert_eq!(
            skeet_auto_band(&[Band::HighQuality, Band::Low]),
            Some(Band::Low)
        );
    }

    #[test]
    fn skeet_auto_band_empty() {
        assert_eq!(skeet_auto_band(&[]), None);
    }

    #[test]
    fn skeet_effective_no_manual() {
        assert_eq!(
            skeet_effective_band(None, Band::MediumHigh),
            Band::MediumHigh
        );
    }

    #[test]
    fn skeet_effective_manual_demote() {
        assert_eq!(
            skeet_effective_band(Some(Band::Low), Band::HighQuality),
            Band::Low
        );
    }

    #[test]
    fn skeet_effective_manual_promote() {
        assert_eq!(
            skeet_effective_band(Some(Band::HighQuality), Band::Low),
            Band::HighQuality
        );
    }

    #[test]
    fn one_bad_image_taints_skeet() {
        let image_bands = vec![Band::HighQuality, Band::Low];
        let auto = skeet_auto_band(&image_bands).expect("non-empty");
        let effective = skeet_effective_band(None, auto);
        assert!(!skeet_visible_in_feed(effective, &image_bands));
    }

    #[test]
    fn manual_skeet_override_beats_image_auto_bands() {
        // Images are all low, but manual skeet override promotes
        let image_bands = vec![Band::Low, Band::MediumLow];
        let auto = skeet_auto_band(&image_bands).expect("non-empty");
        assert_eq!(auto, Band::Low);

        let effective = skeet_effective_band(Some(Band::HighQuality), auto);
        assert_eq!(effective, Band::HighQuality);
        // But individual image bands still block visibility
        assert!(!skeet_visible_in_feed(effective, &image_bands));
    }

    #[test]
    fn all_visible_when_all_good() {
        let image_bands = vec![Band::MediumHigh, Band::HighQuality];
        let auto = skeet_auto_band(&image_bands).expect("non-empty");
        let effective = skeet_effective_band(None, auto);
        assert!(skeet_visible_in_feed(effective, &image_bands));
    }

    #[test]
    fn visible_skeet_blocked_by_bad_image() {
        // Skeet effective is good, but one image is bad
        let image_bands = vec![Band::HighQuality, Band::MediumLow];
        assert!(!skeet_visible_in_feed(Band::HighQuality, &image_bands));
    }
}
