use std::io::Cursor;

use cot::html::Html;
use cot::request::extractors::Path;
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use skeet_store::{DiscoveredAt, ImageId, SkeetId, Zone};
use tracing::{info, instrument, warn};

use crate::Store;

#[derive(Debug)]
pub struct FeedEntry {
    pub discovered_at: String,
    pub image_id: String,
    pub zone: String,
    pub config_version: String,
    pub detected_text: String,
    pub at_uri: String,
    pub web_url: String,
}

pub fn to_feed_entry(
    discovered_at: &DiscoveredAt,
    image_id: &ImageId,
    skeet_id: &SkeetId,
    zone: &Zone,
    config_version: &str,
    detected_text: &str,
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
        detected_text: detected_text.to_string(),
        at_uri: skeet_id.to_string(),
        web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
    })
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

#[instrument(skip_all)]
pub async fn home(Store(store): Store) -> cot::Result<Html> {
    info!("serving home");
    let store_summary = store
        .summarise()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to summarise store: {e}")))?;

    let summary = SummaryView {
        image_count: store_summary.image_count,
        score_count: store_summary.score_count,
        scored_image_count: store_summary.scored_image_count,
        discovered_at_min: store_summary
            .discovered_at_range
            .as_ref()
            .map_or_else(|| "-".to_string(), |(min, _)| min.format_short()),
        discovered_at_max: store_summary
            .discovered_at_range
            .as_ref()
            .map_or_else(|| "-".to_string(), |(_, max)| max.format_short()),
        original_at_min: store_summary
            .original_at_range
            .as_ref()
            .map_or_else(|| "-".to_string(), |(min, _)| min.format_short()),
        original_at_max: store_summary
            .original_at_range
            .as_ref()
            .map_or_else(|| "-".to_string(), |(_, max)| max.format_short()),
    };

    let template = HomeTemplate { summary };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}

#[derive(Debug, Template)]
#[template(path = "feed.html")]
pub struct FeedTemplate {
    pub entries: Vec<FeedEntry>,
}

pub const MAX_FEED_ENTRIES: usize = 50;

#[instrument(skip_all)]
pub async fn latest(Store(store): Store) -> cot::Result<Html> {
    info!("serving latest feed");

    let mut summaries = store
        .list_all_summaries()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))?;

    summaries.sort_by(|a, b| b.discovered_at.cmp(&a.discovered_at));

    let entries: Vec<FeedEntry> = summaries
        .iter()
        .take(MAX_FEED_ENTRIES)
        .filter_map(|img| {
            to_feed_entry(
                &img.discovered_at,
                &img.image_id,
                &img.skeet_id,
                &img.zone,
                img.config_version.as_str(),
                &img.detected_text,
            )
        })
        .collect();

    info!(count = entries.len(), "serving latest feed entries");
    let template = FeedTemplate { entries };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}

#[instrument(skip_all, fields(image_id = %image_id_str))]
pub async fn annotated_image(
    Store(store): Store,
    Path(image_id_str): Path<String>,
) -> cot::Result<Response> {
    info!(image_id = %image_id_str, "serving annotated image");
    let image_id: ImageId = image_id_str
        .parse()
        .map_err(|_| cot::Error::internal(format!("invalid image id: {image_id_str}")))?;

    let stored = store
        .get_by_id(&image_id)
        .await
        .map_err(|e| cot::Error::internal(format!("store error: {e}")))?;

    let Some(stored) = stored else {
        warn!(image_id = %image_id_str, "image not found");
        let mut response = Response::new(Body::fixed("not found"));
        *response.status_mut() = StatusCode::NOT_FOUND;
        return Ok(response);
    };

    let mut buf = Cursor::new(Vec::new());
    stored
        .annotated_image
        .write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| cot::Error::internal(format!("failed to encode image: {e}")))?;

    let mut response = Response::new(Body::fixed(buf.into_inner()));
    response
        .headers_mut()
        .insert("content-type", "image/png".parse().expect("valid header"));
    Ok(response)
}

#[derive(Debug)]
pub struct BestFeedEntry {
    pub entry: FeedEntry,
    pub score: String,
}

#[derive(Debug, Template)]
#[template(path = "best.html")]
pub struct BestTemplate {
    pub entries: Vec<BestFeedEntry>,
}

#[instrument(skip_all)]
pub async fn best(Store(store): Store) -> cot::Result<Html> {
    info!("serving best feed");

    let scored = store
        .list_scored_summaries_by_score()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))?;

    let entries: Vec<BestFeedEntry> = scored
        .iter()
        .take(MAX_FEED_ENTRIES)
        .filter_map(|(img, score)| {
            let entry = to_feed_entry(
                &img.discovered_at,
                &img.image_id,
                &img.skeet_id,
                &img.zone,
                img.config_version.as_str(),
                &img.detected_text,
            )?;
            Some(BestFeedEntry {
                entry,
                score: format!("{}", score),
            })
        })
        .collect();

    info!(count = entries.len(), "serving best feed entries");
    let template = BestTemplate { entries };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}
