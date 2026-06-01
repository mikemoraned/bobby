use serde::{Deserialize, Serialize};
use skeet_store::SkeetId;

use crate::image_url::ImageUrl;

/// One published image: the resolved CDN image URL and the skeet it belongs to.
///
/// Each element of a published redis list is one of these, serialized as a JSON
/// object — `{ "image_url": "...", "skeet_id": "at://..." }` — rather than a
/// delimiter-joined string, because both halves contain `:` (the skeet-id is an
/// AT-URI, the url an https url) and would be ambiguous to split.
///
/// Lists are always per-image: deduplicating to unique skeet-ids (e.g. for
/// `getFeedSkeleton`) is a read-side concern, not part of the stored schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPair {
    pub image_url: ImageUrl,
    pub skeet_id: SkeetId,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PublishedPair {
        PublishedPair {
            image_url: ImageUrl::new(
                "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/bafyfakecid@jpeg",
            )
            .expect("valid url"),
            skeet_id: "at://did:plc:abc/app.bsky.feed.post/rkey1"
                .parse()
                .expect("valid skeet id"),
        }
    }

    #[test]
    fn serializes_as_json_object_with_both_halves() {
        let json: serde_json::Value =
            serde_json::to_value(sample()).expect("serialize to value");
        assert_eq!(
            json["image_url"],
            "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/bafyfakecid@jpeg"
        );
        assert_eq!(json["skeet_id"], "at://did:plc:abc/app.bsky.feed.post/rkey1");
    }

    #[test]
    fn roundtrips_through_json() {
        let pair = sample();
        let encoded = serde_json::to_string(&pair).expect("serialize");
        let decoded: PublishedPair = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, pair);
    }
}
