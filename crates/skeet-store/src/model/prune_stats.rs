use std::ops::AddAssign;

use chrono::{DateTime, Utc};

/// One pruner status-interval's content tallies, timestamped with the wall-clock
/// bounds of the interval they cover.
///
/// Appended once per interval by the pruner and combined by the
/// [`Statistics`](crate::Statistics) port to answer "how much did the pruner see
/// over a window". The three counts mirror the pruner's per-interval content
/// tallies one-to-one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PruneStats {
    /// Start of the interval these counts cover (inclusive).
    pub interval_start: DateTime<Utc>,
    /// End of the interval these counts cover (exclusive).
    pub interval_end: DateTime<Utc>,
    /// Skeets seen on the firehose that reached the image filter.
    pub skeets_seen: u64,
    /// Images examined — looked at by the image filter, before any save.
    pub images_examined: u64,
    /// Images saved as candidates.
    pub images_saved: u64,
}

/// Merge another interval's stats in: the counts sum and the bounds widen to the
/// union span (earliest start, latest end).
///
/// Saturating so the laws hold for all `u64`. Note [`Default`]'s epoch bounds are
/// *not* an identity for the `min`/`max` on the bounds, so fold sums of these
/// must seed from a real interval (e.g. via [`Iterator::reduce`]) rather than
/// from `Default`.
impl AddAssign<&Self> for PruneStats {
    fn add_assign(&mut self, rhs: &Self) {
        self.interval_start = self.interval_start.min(rhs.interval_start);
        self.interval_end = self.interval_end.max(rhs.interval_end);
        self.skeets_seen = self.skeets_seen.saturating_add(rhs.skeets_seen);
        self.images_examined = self.images_examined.saturating_add(rhs.images_examined);
        self.images_saved = self.images_saved.saturating_add(rhs.images_saved);
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, hour, 0, 0).unwrap()
    }

    #[test]
    fn add_assign_sums_counts_and_widens_bounds() {
        let mut acc = PruneStats {
            interval_start: at(1),
            interval_end: at(2),
            skeets_seen: 10,
            images_examined: 4,
            images_saved: 1,
        };
        acc += &PruneStats {
            interval_start: at(3),
            interval_end: at(4),
            skeets_seen: 20,
            images_examined: 6,
            images_saved: 2,
        };
        assert_eq!(
            acc,
            PruneStats {
                // Bounds widen to the union: earliest start, latest end.
                interval_start: at(1),
                interval_end: at(4),
                skeets_seen: 30,
                images_examined: 10,
                images_saved: 3,
            }
        );
    }

    #[test]
    fn add_assign_widens_bounds_regardless_of_merge_order() {
        // Merging an earlier interval into a later one still yields the union.
        let mut acc = PruneStats {
            interval_start: at(5),
            interval_end: at(6),
            ..PruneStats::default()
        };
        acc += &PruneStats {
            interval_start: at(2),
            interval_end: at(3),
            ..PruneStats::default()
        };
        assert_eq!(acc.interval_start, at(2));
        assert_eq!(acc.interval_end, at(6));
    }
}
