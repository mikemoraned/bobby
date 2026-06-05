use std::io::Cursor;

use cot::html::Html;
use cot::http::HeaderValue;
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::Path;
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use shared::{Band, ImageId};
use tracing::{info, instrument, warn};

use crate::AppraiserExtractor;
use crate::PublishedFeedExtractor;
use crate::Store;
use crate::feed_snapshot::FeedSnapshot;

pub struct HomeEntry {
    pub image_id: String,
    pub skeet_id_encoded: String,
    pub image_id_encoded: String,
    pub score: String,
    pub band: String,
    pub manual_skeet_band: String,
    pub manual_image_band: String,
    pub web_url: String,
}

pub struct BandOption {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

/// Bands ordered best-to-worst for UI button display.
const BANDS_BEST_TO_WORST: &[Band] = &[
    Band::HighQuality,
    Band::MediumHigh,
    Band::MediumLow,
    Band::Low,
];

pub fn band_options() -> Vec<BandOption> {
    BANDS_BEST_TO_WORST
        .iter()
        .map(|&b| BandOption {
            name: b.wire_name(),
            label: b.short_label(),
            description: b.description(),
        })
        .collect()
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    entries: Vec<HomeEntry>,
    is_admin: bool,
    band_options: Vec<BandOption>,
}

#[instrument(skip_all)]
pub async fn home(
    AppraiserExtractor(appraiser): AppraiserExtractor,
    PublishedFeedExtractor(feed): PublishedFeedExtractor,
    Store(store): Store,
) -> cot::Result<Html> {
    info!("serving home");

    let is_admin = appraiser.is_some();

    let snapshot = FeedSnapshot::load(&feed, &store)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to load feed snapshot: {e}")))?;

    let entries: Vec<HomeEntry> = snapshot
        .items
        .into_iter()
        .map(|item| {
            let did = item.skeet_id.did();
            let rkey = item.skeet_id.rkey();
            let image_id = item.image_id.to_string();
            HomeEntry {
                skeet_id_encoded: urlencoding::encode(&item.skeet_id.to_string()).into_owned(),
                image_id_encoded: urlencoding::encode(&image_id).into_owned(),
                image_id,
                score: format!("{}", item.score),
                band: item.effective_band.to_string(),
                manual_skeet_band: item
                    .manual_skeet_band
                    .map(|b| b.to_string())
                    .unwrap_or_default(),
                manual_image_band: item
                    .manual_image_band
                    .map(|b| b.to_string())
                    .unwrap_or_default(),
                web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
            }
        })
        .collect();

    info!(count = entries.len(), is_admin, "serving home entries");
    let rendered = HomeTemplate {
        entries,
        is_admin,
        band_options: band_options(),
    }
    .render()?;
    Ok(Html::new(rendered))
}

#[instrument(skip_all, fields(image_id = %image_id_str))]
pub async fn annotated_image(
    head: RequestHead,
    Store(store): Store,
    crate::StartedAtExtractor(started_at): crate::StartedAtExtractor,
    Path(image_id_str): Path<String>,
) -> cot::Result<Response> {
    info!(image_id = %image_id_str, "serving annotated image");

    let last_modified = started_at.http_date();

    // Conditional GET: if the client already has a copy from this server boot, return 304.
    if let Some(ims) = head
        .headers
        .get("if-modified-since")
        .and_then(|v| v.to_str().ok())
        && ims == last_modified
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        return Ok(response);
    }

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

    let last_modified_value: HeaderValue = last_modified
        .parse()
        .map_err(|e| cot::Error::internal(format!("invalid last-modified header: {e}")))?;
    let mut response = Response::new(Body::fixed(buf.into_inner()));
    response
        .headers_mut()
        .insert("content-type", HeaderValue::from_static("image/png"));
    response
        .headers_mut()
        .insert("last-modified", last_modified_value);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Band;

    #[test]
    fn band_options_covers_all_bands() {
        let options = band_options();
        assert_eq!(options.len(), Band::ALL.len());
    }

    #[test]
    fn band_options_names_are_distinct() {
        let options = band_options();
        let unique: std::collections::HashSet<_> = options.iter().map(|o| o.name).collect();
        assert_eq!(unique.len(), options.len());
    }

    #[test]
    fn band_options_ordered_best_to_worst() {
        let options = band_options();
        let bands: Vec<Band> = options
            .iter()
            .map(|o| o.name.parse().expect("valid band"))
            .collect();
        for w in bands.windows(2) {
            assert!(w[0] > w[1]);
        }
    }
}
