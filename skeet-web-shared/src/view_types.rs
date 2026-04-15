use cot::Template;
use skeet_store::{DiscoveredAt, ImageId, SkeetId, Zone};

#[derive(Debug)]
pub struct FeedEntry {
    pub discovered_at: String,
    pub image_id: String,
    pub zone: String,
    pub config_version: String,
    pub at_uri: String,
    pub web_url: String,
}

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

#[derive(Debug)]
pub struct InspectEntry {
    pub entry: FeedEntry,
    pub score: String,
}

#[derive(Debug)]
pub struct SummaryView {
    pub image_count: usize,
    pub score_count: usize,
    pub scored_image_count: usize,
    pub discovered_at_min: String,
    pub discovered_at_max: String,
    pub original_at_min: String,
    pub original_at_max: String,
}

#[derive(Debug, Template)]
#[template(path = "home.html")]
pub struct HomeTemplate {
    pub summary: SummaryView,
}

#[derive(Debug, Template)]
#[template(path = "inspect.html")]
pub struct InspectTemplate {
    pub title: String,
    pub empty_message: String,
    pub entries: Vec<InspectEntry>,
}

pub const MAX_ENTRIES: usize = 50;
