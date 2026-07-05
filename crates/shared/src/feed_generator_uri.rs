use std::fmt;

use crate::skeet_id::Did;

/// The AT-URI of a Bluesky feed generator record:
/// `at://{did}/app.bsky.feed.generator/{feed_name}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedGeneratorUri(String);

const FEED_GENERATOR_COLLECTION: &str = "app.bsky.feed.generator";

impl FeedGeneratorUri {
    /// The feed generator URI for the feed named `feed_name` published under `did`.
    pub fn new(did: &Did, feed_name: &str) -> Self {
        Self(format!("at://{did}/{FEED_GENERATOR_COLLECTION}/{feed_name}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FeedGeneratorUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_at_uri_from_did_and_feed_name() {
        let did = Did::new("did:web:bobby.example.com").expect("valid did");
        let uri = FeedGeneratorUri::new(&did, "bobby-dev");
        assert_eq!(
            uri.as_str(),
            "at://did:web:bobby.example.com/app.bsky.feed.generator/bobby-dev"
        );
    }
}
