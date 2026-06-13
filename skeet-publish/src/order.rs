use std::fmt;
use std::str::FromStr;

/// How a published list is ordered — the first component of its `{order}-{limit}`
/// name.
///
/// `Recency` orders by skeet publish time; `Quality` orders by effective band then
/// normalised score (best first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Order {
    Recency,
    Quality,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid order: \"{0}\"")]
pub struct InvalidOrder(String);

impl fmt::Display for Order {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Recency => "recency",
            Self::Quality => "quality",
        })
    }
}

impl FromStr for Order {
    type Err = InvalidOrder;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "recency" => Ok(Self::Recency),
            "quality" => Ok(Self::Quality),
            other => Err(InvalidOrder(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_lowercase() {
        assert_eq!(Order::Recency.to_string(), "recency");
        assert_eq!(Order::Quality.to_string(), "quality");
    }

    #[test]
    fn roundtrips_through_display() {
        for order in [Order::Recency, Order::Quality] {
            let parsed: Order = order.to_string().parse().expect("roundtrip");
            assert_eq!(parsed, order);
        }
    }

    #[test]
    fn rejects_unknown_order() {
        assert!("foop".parse::<Order>().is_err());
        assert!("".parse::<Order>().is_err());
    }
}
