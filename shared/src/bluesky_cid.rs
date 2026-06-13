use std::fmt;
use std::str::FromStr;

use cid::Cid;

/// A content identifier for a Bluesky image blob, as it appears in a blob ref
/// and in the CDN URL (`https://cdn.bsky.app/img/.../{did}/{cid}@jpeg`).
///
/// Validated on construction by the same `cid` parser atrium uses for blob refs,
/// so any value extracted from the firehose round-trips through its canonical
/// multibase string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlueskyCid(Cid);

#[derive(Debug, thiserror::Error)]
#[error("invalid Bluesky CID: \"{0}\"")]
pub struct InvalidBlueskyCid(String);

impl BlueskyCid {
    /// Validating constructor for untrusted input (e.g. a parsed image id).
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidBlueskyCid> {
        let s = s.into();
        Cid::from_str(&s).map(Self).map_err(|_| InvalidBlueskyCid(s))
    }
}

impl FromStr for BlueskyCid {
    type Err = InvalidBlueskyCid;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for BlueskyCid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Bluesky blob CID (CIDv1, base32, sha2-256).
    const SAMPLE: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

    #[test]
    fn roundtrips_through_display() {
        let cid = BlueskyCid::new(SAMPLE).expect("valid cid");
        assert_eq!(cid.to_string(), SAMPLE);
        let parsed: BlueskyCid = SAMPLE.parse().expect("valid cid");
        assert_eq!(parsed, cid);
    }

    #[test]
    fn rejects_non_cid() {
        assert!(BlueskyCid::new("not-a-cid").is_err());
        assert!("".parse::<BlueskyCid>().is_err());
    }
}
