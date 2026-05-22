use std::fmt;

use chrono::{DateTime, Utc};
use image::DynamicImage;
use shared::{ImageId, ModelVersion};
pub use shared::Zone;
pub use shared::skeet_id::SkeetId;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoveredAt(DateTime<Utc>);

impl DiscoveredAt {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub const fn new(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub const fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    pub fn format_short(&self) -> String {
        self.0.format("%Y-%m-%d %H:%M").to_string()
    }

    pub fn is_within_hours(&self, now: DateTime<Utc>, hours: u64) -> bool {
        let cutoff = now - chrono::Duration::hours(hours as i64);
        self.0 >= cutoff
    }
}

impl fmt::Display for DiscoveredAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OriginalAt(DateTime<Utc>);

impl OriginalAt {
    pub const fn new(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub const fn timestamp_micros(&self) -> i64 {
        self.0.timestamp_micros()
    }

    pub fn format_short(&self) -> String {
        self.0.format("%Y-%m-%d %H:%M").to_string()
    }
}

impl fmt::Display for OriginalAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct ImageRecord {
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub image: DynamicImage,
    pub discovered_at: DiscoveredAt,
    pub original_at: OriginalAt,
    pub zone: Zone,
    pub annotated_image: DynamicImage,
    pub config_version: ModelVersion,
    pub detected_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn discovered_at_formatting() {
        use chrono::TimeZone as _;
        let dt = chrono::Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        let d = DiscoveredAt::new(dt);
        assert_eq!(d.format_short(), "2024-06-15 09:30");
        assert!(d.to_string().contains("2024-06-15"));
    }

    #[test]
    fn original_at_formatting_and_timestamp() {
        use chrono::TimeZone as _;
        let dt = chrono::Utc.with_ymd_and_hms(2024, 6, 15, 9, 30, 0).unwrap();
        let o = OriginalAt::new(dt);
        assert_eq!(o.timestamp_micros(), dt.timestamp_micros());
        assert_eq!(o.format_short(), "2024-06-15 09:30");
        assert!(o.to_string().contains("2024-06-15"));
    }

    proptest! {
        /// A timestamp equal to `now` is always within any hour window (offset = 0).
        #[test]
        fn discovered_now_is_always_within(
            ts in 0i64..=2_000_000_000i64,
            hours in 0u64..=8760u64,
        ) {
            use chrono::TimeZone as _;
            let now = chrono::Utc.timestamp_opt(ts, 0).single()
                .expect("ts in range");
            let d = DiscoveredAt::new(now);
            prop_assert!(d.is_within_hours(now, hours));
        }

        /// `is_within_hours` matches the arithmetic: offset_hours <= window_hours ⟺ within.
        #[test]
        fn within_hours_matches_arithmetic(
            ts_now in 360_000i64..=2_000_000_000i64,
            offset_hours in 0u64..=100u64,
            window_hours in 0u64..=200u64,
        ) {
            use chrono::TimeZone as _;
            let now = chrono::Utc.timestamp_opt(ts_now, 0).single()
                .expect("ts in range");
            let then = chrono::Utc
                .timestamp_opt(ts_now - offset_hours as i64 * 3600, 0)
                .single()
                .expect("ts in range");
            let d = DiscoveredAt::new(then);
            prop_assert_eq!(d.is_within_hours(now, window_hours), offset_hours <= window_hours);
        }
    }
}
