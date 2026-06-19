use crate::limit::{InvalidLimit, Limit};
use crate::order::{InvalidOrder, Order};

/// Error parsing a published-list spec.
#[derive(Debug, thiserror::Error)]
pub enum InvalidSpec {
    #[error("expected <order>-<limit> (e.g. quality-48h), got \"{0}\"")]
    Malformed(String),
    #[error(transparent)]
    Order(#[from] InvalidOrder),
    #[error(transparent)]
    Limit(#[from] InvalidLimit),
}

/// Parse a published-list spec `<order>-<limit>` (e.g. `quality-48h`, `recency-48h`,
/// `quality-7d`) into its `(Order, Limit)`.
///
/// The inverse of the `{order}-{limit}` naming used both for redis list names and
/// for the publisher's / reader's `--publish` args. Order and limit tokens never
/// contain `-`, so the split is unambiguous.
pub fn parse_spec(s: &str) -> Result<(Order, Limit), InvalidSpec> {
    let (order, limit) = s
        .split_once('-')
        .ok_or_else(|| InvalidSpec::Malformed(s.to_string()))?;
    Ok((order.parse()?, limit.parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_specs() {
        assert_eq!(
            parse_spec("quality-48h").expect("valid"),
            (Order::Quality, Limit::hours(48))
        );
        assert_eq!(
            parse_spec("recency-48h").expect("valid"),
            (Order::Recency, Limit::hours(48))
        );
        assert_eq!(
            parse_spec("quality-7d").expect("valid"),
            (Order::Quality, Limit::days(7))
        );
    }

    #[test]
    fn rejects_missing_separator() {
        assert!(matches!(
            parse_spec("quality"),
            Err(InvalidSpec::Malformed(_))
        ));
    }

    #[test]
    fn rejects_bad_order_or_limit() {
        assert!(matches!(
            parse_spec("bogus-48h"),
            Err(InvalidSpec::Order(_))
        ));
        assert!(matches!(
            parse_spec("quality-99x"),
            Err(InvalidSpec::Limit(_))
        ));
    }
}
