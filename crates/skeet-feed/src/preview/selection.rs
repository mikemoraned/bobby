//! Choosing which published images make up the montage, and fingerprinting that
//! choice so identical selections map to the same cached image and ETag.

use bluesky::ImageUrl;
use shared::ImageId;
use skeet_publish::PublishedImage;

/// How many of the best (first) live images go into the montage. Bounded so the
/// fetch and compose work stays flat regardless of feed size.
pub const MONTAGE_TILE_COUNT: usize = 10;

/// A short, stable fingerprint of the exact tiles — and their order — that make
/// up a montage. Equal signatures mean an identical montage, so it doubles as
/// the in-memory cache key and the HTTP `ETag`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentSignature(String);

impl ContentSignature {
    /// Fingerprint an ordered sequence of image ids. Order-sensitive: the same
    /// ids in a different order produce a different signature.
    fn of_ids<'a>(ids: impl IntoIterator<Item = &'a ImageId>) -> Self {
        let mut joined = String::new();
        for id in ids {
            joined.push_str(&id.to_string());
            joined.push('\n');
        }
        Self(format!("{:x}", md5::compute(joined)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The tiles selected for the montage: their thumbnail URLs in best-first order,
/// paired with the signature of that ordered selection.
pub struct MontageSelection {
    pub tile_urls: Vec<ImageUrl>,
    pub signature: ContentSignature,
}

/// Select the montage tiles from a published list.
///
/// Keeps only live items (so a dead thumbnail can never reach the montage),
/// takes the first [`MONTAGE_TILE_COUNT`] in published (best-first) order, and
/// fingerprints their ordered image ids.
pub fn select_tiles(images: &[PublishedImage]) -> MontageSelection {
    let live: Vec<&PublishedImage> = images
        .iter()
        .filter(|image| image.is_live())
        .take(MONTAGE_TILE_COUNT)
        .collect();
    let signature = ContentSignature::of_ids(live.iter().map(|image| &image.image_id));
    let tile_urls = live.iter().map(|image| image.image_url.clone()).collect();
    MontageSelection {
        tile_urls,
        signature,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{BlueskyCid, SkeetId};

    fn skeet_id(rkey: &str) -> SkeetId {
        format!("at://did:plc:abc/app.bsky.feed.post/{rkey}")
            .parse()
            .expect("valid skeet id")
    }

    /// A live published image keyed by a distinct `seed` letter (a→z), which
    /// drives both a unique CID (image id) and thumbnail URL.
    fn live_image(seed: char) -> PublishedImage {
        // A valid base32 CIDv1 with one character swapped per seed.
        let cid = format!("bafkreibme22gw2h7y2h7tg2fhqotaq{seed}ucnbc24deqo72b6mkl2egezxhvy");
        PublishedImage::unprobed(
            ImageUrl::new(format!(
                "https://cdn.bsky.app/img/feed_thumbnail/plain/did:plc:abc/{cid}@jpeg"
            ))
            .expect("valid url"),
            ImageId::V3(BlueskyCid::new(&cid).expect("valid cid")),
            skeet_id(&format!("post{seed}")),
        )
    }

    fn dead_image(seed: char) -> PublishedImage {
        let mut image = live_image(seed);
        image.image_url_exists = false;
        image
    }

    fn images(seeds: &str) -> Vec<PublishedImage> {
        seeds.chars().map(live_image).collect()
    }

    #[test]
    fn same_list_yields_the_same_signature() {
        assert_eq!(
            select_tiles(&images("abcde")).signature,
            select_tiles(&images("abcde")).signature
        );
    }

    #[test]
    fn reordering_changes_the_signature() {
        assert_ne!(
            select_tiles(&images("abcde")).signature,
            select_tiles(&images("abced")).signature
        );
    }

    #[test]
    fn changing_a_tile_changes_the_signature() {
        assert_ne!(
            select_tiles(&images("abcde")).signature,
            select_tiles(&images("abcdf")).signature
        );
    }

    #[test]
    fn dead_items_are_excluded_from_selection_and_signature() {
        let with_dead = {
            let mut list = images("abc");
            list.push(dead_image('z'));
            list
        };
        let selection = select_tiles(&with_dead);
        assert_eq!(selection.tile_urls.len(), 3, "dead tile must be dropped");
        // A trailing dead item can't affect the montage's identity.
        assert_eq!(selection.signature, select_tiles(&images("abc")).signature);
    }

    #[test]
    fn selection_is_capped_at_the_tile_count() {
        // 12 live images, but only the first MONTAGE_TILE_COUNT are used, so a
        // change beyond the cap leaves the signature untouched.
        let twelve = images("abcdefghijkl");
        let selection = select_tiles(&twelve);
        assert_eq!(selection.tile_urls.len(), MONTAGE_TILE_COUNT);

        let mut altered = twelve;
        altered[11] = live_image('z');
        assert_eq!(
            selection.signature,
            select_tiles(&altered).signature,
            "changing an image past the cap must not change the signature"
        );
    }

    #[test]
    fn signature_as_str_is_stable_hex_reflecting_content() {
        let a = select_tiles(&images("abc")).signature;
        let b = select_tiles(&images("abd")).signature;
        assert_eq!(a.as_str().len(), 32, "md5 hex is 32 chars");
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(
            a.as_str(),
            b.as_str(),
            "different content must surface as a different string"
        );
    }

    #[test]
    fn tile_urls_follow_published_order() {
        let selection = select_tiles(&images("abc"));
        let urls: Vec<String> = selection.tile_urls.iter().map(ToString::to_string).collect();
        assert!(urls[0].contains("aucnbc"), "first url is seed a");
        assert!(urls[1].contains("bucnbc"), "second url is seed b");
    }
}
