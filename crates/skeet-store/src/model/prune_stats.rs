use chrono::{DateTime, Utc};

/// One pruner status-interval's content tallies, timestamped with the wall-clock
/// bounds of the interval they cover.
///
/// Appended once per interval by the pruner and summed by the
/// [`Statistics`](crate::Statistics) port to answer "how many images were
/// examined over a window". The three counts mirror the pruner's per-interval
/// content tallies one-to-one.
#[derive(Debug, Clone, PartialEq, Eq)]
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
