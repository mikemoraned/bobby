use std::io::Cursor;

use cot::html::Html;
use cot::request::extractors::Path;
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use skeet_store::ImageId;
use skeet_web_shared::{FeedEntry, Store, to_feed_entry};
use tracing::{info, instrument, warn};

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
#[template(path = "inspect.html")]
pub struct InspectTemplate {
    pub title: String,
    pub empty_message: String,
    pub entries: Vec<InspectEntry>,
}

pub const MAX_ENTRIES: usize = 50;

#[instrument(skip_all)]
pub async fn pruned(Store(store): Store) -> cot::Result<Html> {
    info!("serving pruned page");

    let mut summaries = store
        .list_all_summaries()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))?;

    summaries.sort_by(|a, b| b.discovered_at.cmp(&a.discovered_at));
    summaries.truncate(MAX_ENTRIES);

    let image_id_strings: Vec<String> = summaries
        .iter()
        .map(|s| s.image_id.to_string())
        .collect();
    let image_ids: Vec<&str> = image_id_strings.iter().map(|s| s.as_str()).collect();
    let score_map = store
        .list_scores_for_ids(&image_ids)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read scores: {e}")))?;

    let entries: Vec<InspectEntry> = summaries
        .iter()
        .filter_map(|img| {
            let entry = to_feed_entry(
                &img.discovered_at,
                &img.image_id,
                &img.skeet_id,
                &img.zone,
                img.config_version.as_str(),
            )?;
            let score = score_map
                .get(&img.image_id)
                .map(|s| format!("{s}"))
                .unwrap_or_else(|| "None".to_string());
            Some(InspectEntry { entry, score })
        })
        .collect();

    info!(count = entries.len(), "serving pruned entries");
    let template = InspectTemplate {
        title: "Pruned Skeets".to_string(),
        empty_message: "No skeets found yet. Run <code>just prune</code> to start collecting."
            .to_string(),
        entries,
    };
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

#[instrument(skip_all)]
pub async fn refined(Store(store): Store) -> cot::Result<Html> {
    info!("serving refined page");

    let scored = store
        .list_scored_summaries_by_score(MAX_ENTRIES, None)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read scores: {e}")))?;

    let entries: Vec<InspectEntry> = scored
        .iter()
        .take(MAX_ENTRIES)
        .filter_map(|(img, score)| {
            let entry = to_feed_entry(
                &img.discovered_at,
                &img.image_id,
                &img.skeet_id,
                &img.zone,
                img.config_version.as_str(),
            )?;
            Some(InspectEntry {
                entry,
                score: format!("{score}"),
            })
        })
        .collect();

    info!(count = entries.len(), "serving refined entries");
    let template = InspectTemplate {
        title: "Refined Skeets".to_string(),
        empty_message: "No scored skeets yet.".to_string(),
        entries,
    };
    let rendered = template.render()?;
    Ok(Html::new(rendered))
}
