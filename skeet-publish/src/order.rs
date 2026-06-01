use std::fmt;
use std::str::FromStr;

/// How a published list is ordered — the first component of its `{order}-{limit}`
/// name.
///
/// Only `Recency` (by skeet publish time) exists today; modelled as an enum so a
/// `Quality` ordering (by score/band) can be added later without changing the
/// naming scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Order {
    Recency,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid order: \"{0}\"")]
pub struct InvalidOrder(String);

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Recency => "recency",
        })
    }
}

impl FromStr for Order {
    type Err = InvalidOrder;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "recency" => Ok(Self::Recency),
            other => Err(InvalidOrder(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_displays_lowercase() {
        assert_eq!(Order::Recency.to_string(), "recency");
    }

    #[test]
    fn roundtrips_through_display() {
        let parsed: Order = Order::Recency.to_string().parse().expect("roundtrip");
        assert_eq!(parsed, Order::Recency);
    }

    #[test]
    fn rejects_unknown_order() {
        assert!("foop".parse::<Order>().is_err());
        assert!("".parse::<Order>().is_err());
    }
}
