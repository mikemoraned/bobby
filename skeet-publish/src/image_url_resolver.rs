use shared::ImageId;
use skeet_store::SkeetId;

use crate::image_url::ImageUrl;

/// Resolves a published image to its public image URL.
///
/// Hides where the URL comes from — a `v3:` CID, a stored url, or elsewhere — so
/// the publisher writes already-resolved URLs and `skeet-feed` never has to
/// know. Returns `None` when no URL can be produced for the image.
pub trait ImageUrlResolver: Send + Sync {
    fn resolve(&self, skeet_id: &SkeetId, image_id: &ImageId) -> Option<ImageUrl>;
}

/// Resolver producing the Bluesky CDN thumbnail URL
/// (`https://cdn.bsky.app/img/feed_thumbnail/plain/{did}/{cid}@jpeg`) from the
/// `did` in the skeet-id and the `cid` carried by an [`ImageId::V3`].
///
/// Returns `None` for V1/V2 ids: they key on image content, not the uploaded
/// blob, so they carry no recoverable CID.
pub struct CdnImageUrlResolver;

impl ImageUrlResolver for CdnImageUrlResolver {
    fn resolve(&self, skeet_id: &SkeetId, image_id: &ImageId) -> Option<ImageUrl> {
        let ImageId::V3(cid) = image_id else {
            return None;
        };
        ImageUrl::new(format!(
            "https://cdn.bsky.app/img/feed_thumbnail/plain/{}/{cid}@jpeg",
            skeet_id.did()
        ))
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::BlueskyCid;

    const SAMPLE_CID: &str = "bafkreibme22gw2h7y2h7tg2fhqotaqjucnbc24deqo72b6mkl2egezxhvy";

    fn skeet_id() -> SkeetId {
        "at://did:plc:abc123/app.bsky.feed.post/rkey1"
            .parse()
            .expect("valid skeet id")
    }

    #[test]
    fn resolves_v3_to_cdn_thumbnail() {
        let image_id = ImageId::V3(BlueskyCid::new(SAMPLE_CID).expect("valid cid"));
        let url = CdnImageUrlResolver
            .resolve(&skeet_id(), &image_id)
            .expect("v3 resolves");
        assert_eq!(
            url.as_str(),
            format!("https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc123/{SAMPLE_CID}@jpeg")
        );
    }

    #[test]
    fn returns_none_for_non_v3() {
        let v2: ImageId = "v2:0123456789abcdef0123456789abcdef"
            .parse()
            .expect("valid v2 id");
        assert!(CdnImageUrlResolver.resolve(&skeet_id(), &v2).is_none());
    }
}
