use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;

/// The recency window of a published list — the second component of its
/// `{order}-{limit}` name (e.g. `48h`, `7d`, `365d`).
///
/// The original unit is preserved rather than normalised to a `Duration`, so
/// `48h` renders as `48h` and not `2d`: the rendered form *is* the redis list
/// name and must be stable. The `<count><unit>` grammar is a fixed,
/// application-specific naming scheme (not a general human-duration format), so
/// it is parsed directly rather than via a duration crate, none of which
/// round-trips the chosen unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limit {
    count: u64,
    unit: Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Unit {
    Hours,
    Days,
}

impl Unit {
    const fn suffix(self) -> char {
        match self {
            Self::Hours => 'h',
            Self::Days => 'd',
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown limit unit '{0}' (expected 'h' or 'd')")]
struct UnknownUnit(char);

impl TryFrom<char> for Unit {
    type Error = UnknownUnit;

    fn try_from(c: char) -> Result<Self, Self::Error> {
        match c {
            'h' => Ok(Self::Hours),
            'd' => Ok(Self::Days),
            other => Err(UnknownUnit(other)),
        }
    }
}

/// A whole `<count><unit>` limit: a run of digits followed by a single `h`/`d`.
#[allow(clippy::expect_used)] // compile-time-constant regex literal
static LIMIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([0-9]+)([hd])$").expect("static regex"));

#[derive(Debug, thiserror::Error)]
#[error("invalid limit: \"{0}\" (expected <count><h|d>, e.g. 48h or 7d)")]
pub struct InvalidLimit(String);

impl TryFrom<&str> for Limit {
    type Error = InvalidLimit;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let invalid = || InvalidLimit(s.to_string());
        let caps = LIMIT_RE.captures(s).ok_or_else(invalid)?;
        let count: u64 = caps[1].parse().map_err(|_| invalid())?;
        if count == 0 {
            return Err(invalid());
        }
        // The regex restricts group 2 to a single `h`/`d`, which `Unit::try_from`
        // turns into the enum.
        let unit_char = caps[2].chars().next().ok_or_else(invalid)?;
        let unit = Unit::try_from(unit_char).map_err(|_| invalid())?;
        Ok(Self { count, unit })
    }
}

impl Limit {
    /// Validating constructor for untrusted input (e.g. a `--publish` arg).
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidLimit> {
        Self::try_from(s.into().as_str())
    }

    pub const fn hours(count: u64) -> Self {
        Self {
            count,
            unit: Unit::Hours,
        }
    }

    pub const fn days(count: u64) -> Self {
        Self {
            count,
            unit: Unit::Days,
        }
    }

    /// The window as a `chrono::Duration` for filtering by age.
    pub const fn window(self) -> chrono::Duration {
        match self.unit {
            Unit::Hours => chrono::Duration::hours(self.count as i64),
            Unit::Days => chrono::Duration::days(self.count as i64),
        }
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.count, self.unit.suffix())
    }
}

impl FromStr for Limit {
    type Err = InvalidLimit;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_with_original_unit() {
        assert_eq!(Limit::hours(48).to_string(), "48h");
        assert_eq!(Limit::days(7).to_string(), "7d");
        assert_eq!(Limit::days(365).to_string(), "365d");
    }

    #[test]
    fn forty_eight_hours_is_not_normalised_to_days() {
        // 48h and 2d are the same duration but must not share a name.
        assert_ne!(Limit::hours(48).to_string(), Limit::days(2).to_string());
        assert_eq!(Limit::hours(48).window(), Limit::days(2).window());
    }

    #[test]
    fn roundtrips_through_display() {
        for limit in [Limit::hours(48), Limit::days(7), Limit::days(365)] {
            let parsed: Limit = limit.to_string().parse().expect("roundtrip");
            assert_eq!(parsed, limit);
        }
    }

    #[test]
    fn window_matches_unit() {
        assert_eq!(Limit::hours(48).window(), chrono::Duration::hours(48));
        assert_eq!(Limit::days(7).window(), chrono::Duration::days(7));
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "h", "d", "48", "48m", "0h", "-1h", "hh", "4.5d"] {
            assert!(bad.parse::<Limit>().is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn unit_try_from_recognises_known_suffixes() {
        assert_eq!(Unit::try_from('h').expect("hours"), Unit::Hours);
        assert_eq!(Unit::try_from('d').expect("days"), Unit::Days);
        assert!(Unit::try_from('m').is_err());
    }
}
