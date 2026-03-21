use std::io::Cursor;

use cot::html::Html;
use cot::request::extractors::Path;
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use skeet_store::{ImageId, SkeetId, Zone};
use tracing::{info, instrument, warn};

use crate::STORE_ARGS;

#[derive(Debug)]
pub struct FeedEntry {
    pub image_id: String,
    pub zone: String,
    pub config_version: String,
    pub detected_text: String,
    pub at_uri: String,
    pub web_url: String,
}

pub fn to_feed_entry(
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
        image_id: image_id.to_string(),
        zone: zone.to_string(),
        config_version: config_version.to_string(),
        detected_text: detected_text.to_string(),
        at_uri: skeet_id.to_string(),
        web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
    })
}

#[derive(Debug, Template)]
#[template(path = "feed.html")]
pub struct FeedTemplate {
    pub entries: Vec<FeedEntry>,
}

pub const MAX_FEED_ENTRIES: usize = 50;

#[instrument]
pub async fn feed() -> cot::Result<Html> {
    info!("serving feed");
    let store = open_store().await?;

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
                &img.image_id,
                &img.skeet_id,
                &img.zone,
                img.config_version.as_str(),
                &img.detected_text,
            )
        })
        .collect();

    info!(count = entries.len(), "serving feed entries");
    let template = FeedTemplate { entries };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}

#[instrument(skip_all, fields(image_id = %image_id_str))]
pub async fn annotated_image(Path(image_id_str): Path<String>) -> cot::Result<Response> {
    info!(image_id = %image_id_str, "serving annotated image");
    let image_id: ImageId = image_id_str
        .parse()
        .map_err(|_| cot::Error::internal(format!("invalid image id: {image_id_str}")))?;

    let store = open_store().await?;
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

#[instrument]
async fn open_store() -> cot::Result<skeet_store::SkeetStore> {
    let store_args = STORE_ARGS
        .get()
        .ok_or_else(|| cot::Error::internal("store args not initialized"))?;
    store_args
        .open_store()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to open store: {e}")))
}
