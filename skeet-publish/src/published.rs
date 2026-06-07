use serde::{Deserialize, Serialize};
use shared::ImageId;
use skeet_store::SkeetId;

use crate::image_url::ImageUrl;

/// Version of the on-the-wire `PublishedImage` JSON, prefixed onto every redis list name.
///
/// Bump it whenever `PublishedImage`'s serialization changes incompatibly, so a new
/// writer and an old (still-deployed) reader use different keys (`v2-recency-48h`)
/// instead of one side failing to decode the other's format.
///
/// - `v1`: `{ image_url, skeet_id }`
/// - `v2`: added `image_id`
pub const SCHEMA_VERSION: &str = "v2";

/// One published image: the resolved CDN image URL, the image's id, and the
/// skeet it belongs to.
///
/// Each element of a published redis list is one of these, serialized as a JSON
/// object — `{ "image_url": "...", "image_id": "v3:...", "skeet_id": "at://..." }`
/// — rather than a delimiter-joined string, because the parts contain `:`
/// (the skeet-id is an AT-URI, the url an https url) and would be ambiguous to
/// split. The `image_id` lets a reader join back to live store detail (score,
/// appraisals) for the published item.
///
/// Lists are always per-image: deduplicating to unique skeet-ids (e.g. for
/// `getFeedSkeleton`) is a read-side concern, not part of the stored schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedImage {
    pub image_url: ImageUrl,
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::BlueskyCid;

    const CID: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

    fn sample() -> PublishedImage {
        PublishedImage {
            image_url: ImageUrl::new(format!(
                "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{CID}@jpeg"
            ))
            .expect("valid url"),
            image_id: ImageId::V3(BlueskyCid::new(CID).expect("valid cid")),
            skeet_id: "at://did:plc:abc/app.bsky.feed.post/rkey1"
                .parse()
                .expect("valid skeet id"),
        }
    }

    #[test]
    fn serializes_as_json_object_with_all_parts() {
        let json: serde_json::Value = serde_json::to_value(sample()).expect("serialize to value");
        assert_eq!(
            json["image_url"],
            format!("https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{CID}@jpeg")
        );
        assert_eq!(json["image_id"], format!("v3:{CID}"));
        assert_eq!(json["skeet_id"], "at://did:plc:abc/app.bsky.feed.post/rkey1");
    }

    #[test]
    fn roundtrips_through_json() {
        let item = sample();
        let encoded = serde_json::to_string(&item).expect("serialize");
        let decoded: PublishedImage = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, item);
    }
}
