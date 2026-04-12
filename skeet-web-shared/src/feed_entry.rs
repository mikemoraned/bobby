use skeet_store::{DiscoveredAt, ImageId, SkeetId, Zone};

/// A row of feed/admin UI data: formatted strings ready to render.
#[derive(Debug)]
pub struct FeedEntry {
    pub discovered_at: String,
    pub image_id: String,
    pub zone: String,
    pub config_version: String,
    pub at_uri: String,
    pub web_url: String,
}

/// Build a [`FeedEntry`] from store types, returning `None` for non-post AT URIs.
pub fn to_feed_entry(
    discovered_at: &DiscoveredAt,
    image_id: &ImageId,
    skeet_id: &SkeetId,
    zone: &Zone,
    config_version: &str,
) -> Option<FeedEntry> {
    if skeet_id.collection() != "app.bsky.feed.post" {
        return None;
    }
    let did = skeet_id.did();
    let rkey = skeet_id.rkey();
    Some(FeedEntry {
        discovered_at: discovered_at.format_short(),
        image_id: image_id.to_string(),
        zone: zone.to_string(),
        config_version: config_version.to_string(),
        at_uri: skeet_id.to_string(),
        web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
    })
}

#[cfg(test)]
mod tests {
    use image::DynamicImage;

    use super::*;

    #[test]
    fn converts_at_uri_to_entry() {
        let discovered_at = DiscoveredAt::now();
        let image_id = ImageId::from_image(&DynamicImage::new_rgba8(1, 1));
        let skeet_id: SkeetId = "at://did:plc:abc123/app.bsky.feed.post/xyz789"
            .parse()
            .expect("valid AT URI");
        let zone = Zone::TopRight;
        let entry = to_feed_entry(&discovered_at, &image_id, &skeet_id, &zone, "v1")
            .expect("should produce entry");
        assert_eq!(entry.at_uri, "at://did:plc:abc123/app.bsky.feed.post/xyz789");
        assert_eq!(
            entry.web_url,
            "https://bsky.app/profile/did:plc:abc123/post/xyz789"
        );
    }

    #[test]
    fn returns_none_for_non_post_uri() {
        let discovered_at = DiscoveredAt::now();
        let image_id = ImageId::from_image(&DynamicImage::new_rgba8(1, 1));
        let skeet_id: SkeetId = "at://did:plc:abc123/app.bsky.feed.like/xyz789"
            .parse()
            .expect("valid AT URI");
        let zone = Zone::TopRight;
        assert!(to_feed_entry(&discovered_at, &image_id, &skeet_id, &zone, "v1").is_none());
    }
}
