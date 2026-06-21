use bluesky::{Dimensions, ImageUrl};
use serde::{Deserialize, Serialize};
use shared::{ImageId, SkeetId};

/// Version of the on-the-wire `PublishedImage` JSON, prefixed onto every redis list name.
///
/// Bump it whenever `PublishedImage`'s serialization changes incompatibly, so a new
/// writer and an old (still-deployed) reader use different keys (`v3-recency-48h`)
/// instead of one side failing to decode the other's format.
///
/// - `v1`: `{ image_url, skeet_id }`
/// - `v2`: added `image_id`
/// - `v3`: added `skeet_id_exists`, `image_url_exists`, `image_url_dimensions`
pub const SCHEMA_VERSION: &str = "v3";

/// One published image plus the publisher's last existence verdict.
///
/// Carries the resolved CDN image URL, the image's id, the skeet it belongs to,
/// whether the skeet/image still exist, and the image's dimensions (learned
/// during the existence probe).
///
/// Each element of a published redis list is one of these, serialized as a JSON
/// object, rather than a delimiter-joined string, because the parts contain `:`
/// (the skeet-id is an AT-URI, the url an https url) and would be ambiguous to
/// split. The `image_id` lets a reader join back to live store detail (score,
/// appraisals) for the published item.
///
/// `skeet_id_exists` / `image_url_exists` are the publisher's existence verdict
/// (fail-open: a probe that can't conclude leaves them `true`); readers either
/// filter on them or annotate with them. `image_url_dimensions` is `None` until
/// a successful probe has measured the image.
///
/// Lists are always per-image: deduplicating to unique skeet-ids (e.g. for
/// `getFeedSkeleton`) is a read-side concern, not part of the stored schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedImage {
    pub image_url: ImageUrl,
    pub image_id: ImageId,
    pub skeet_id: SkeetId,
    pub skeet_id_exists: bool,
    pub image_url_exists: bool,
    pub image_url_dimensions: Option<Dimensions>,
}

impl PublishedImage {
    /// A published image not yet probed for existence: both `*_exists` flags
    /// default to `true` (fail-open — shown until a probe proves otherwise) and
    /// dimensions are unknown. The publisher overwrites these from its existence
    /// checker before writing the list.
    pub const fn unprobed(image_url: ImageUrl, image_id: ImageId, skeet_id: SkeetId) -> Self {
        Self {
            image_url,
            image_id,
            skeet_id,
            skeet_id_exists: true,
            image_url_exists: true,
            image_url_dimensions: None,
        }
    }
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
            skeet_id_exists: true,
            image_url_exists: false,
            image_url_dimensions: Some(Dimensions {
                width: 800,
                height: 600,
            }),
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
        assert_eq!(
            json["skeet_id"],
            "at://did:plc:abc/app.bsky.feed.post/rkey1"
        );
        assert_eq!(json["skeet_id_exists"], true);
        assert_eq!(json["image_url_exists"], false);
        assert_eq!(json["image_url_dimensions"]["width"], 800);
        assert_eq!(json["image_url_dimensions"]["height"], 600);
    }

    #[test]
    fn unprobed_defaults_to_present_with_unknown_dimensions() {
        let item = PublishedImage::unprobed(
            ImageUrl::new(format!(
                "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{CID}@jpeg"
            ))
            .expect("valid url"),
            ImageId::V3(BlueskyCid::new(CID).expect("valid cid")),
            "at://did:plc:abc/app.bsky.feed.post/rkey1"
                .parse()
                .expect("valid skeet id"),
        );
        assert!(item.skeet_id_exists);
        assert!(item.image_url_exists);
        assert_eq!(item.image_url_dimensions, None);
    }

    #[test]
    fn roundtrips_through_json() {
        let item = sample();
        let encoded = serde_json::to_string(&item).expect("serialize");
        let decoded: PublishedImage = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, item);
    }
}
