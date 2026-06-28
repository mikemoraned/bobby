use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-list aggregate statistics published alongside a feed list, so a consumer
/// can render "N images checked over this window, of which M shown".
///
/// Written to a list's `{name}:statistics` companion key as a JSON object (see
/// [`PublishedList::write_statistics`](crate::PublishedList::write_statistics)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListStatistics {
    /// Start of the window the list covers (inclusive).
    pub interval_start: DateTime<Utc>,
    /// End of the window the list covers (exclusive).
    pub interval_end: DateTime<Utc>,
    /// Images the pruner examined over the window.
    pub examined: u64,
    /// Images found to match — the full published list length, including
    /// candidates the publisher's existence probe has since found deleted.
    pub found: u64,
    /// Of `found`, how many are still live ([`crate::PublishedImage::is_live`]) —
    /// what the feed actually shows. The match count the public banner reports.
    pub exists: u64,
}

impl ListStatistics {
    pub const fn new(
        interval_start: DateTime<Utc>,
        interval_end: DateTime<Utc>,
        examined: u64,
        found: u64,
        exists: u64,
    ) -> Self {
        Self {
            interval_start,
            interval_end,
            examined,
            found,
            exists,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, hour, 0, 0).unwrap()
    }

    #[test]
    fn roundtrips_through_json() {
        let stats = ListStatistics::new(at(0), at(12), 400_000, 46, 44);
        let encoded = serde_json::to_string(&stats).expect("serialize");
        let decoded: ListStatistics = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, stats);
    }

    #[test]
    fn serializes_as_json_object_with_all_parts() {
        let stats = ListStatistics::new(at(0), at(12), 400_000, 46, 44);
        let json: serde_json::Value = serde_json::to_value(&stats).expect("to value");
        assert_eq!(json["interval_start"], "2026-06-01T00:00:00Z");
        assert_eq!(json["interval_end"], "2026-06-01T12:00:00Z");
        assert_eq!(json["examined"], 400_000);
        assert_eq!(json["found"], 46);
        assert_eq!(json["exists"], 44);
    }
}
