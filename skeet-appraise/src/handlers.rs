use std::io::Cursor;
use std::sync::Arc;

use cot::html::Html;
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::Path;
use cot::response::Response;
use cot::{Body, StatusCode, Template};
use shared::{Band, ImageId};
use skeet_publish::effective_band::image_effective_band;
use skeet_publish::{CachedFeed, FeedCache, visible_entries};
use tracing::{info, instrument, warn};

use crate::AppraiserExtractor;
use crate::FeedCacheExtractor;
use crate::Store;

fn wants_no_cache(head: &RequestHead) -> bool {
    head.headers
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("no-cache"))
}

async fn get_feed(cache: &Arc<FeedCache>, head: &RequestHead) -> cot::Result<CachedFeed> {
    if wants_no_cache(head) {
        info!("cache-control: no-cache — forcing refresh");
        cache
            .refresh()
            .await
            .map_err(|e| cot::Error::internal(format!("failed to refresh cache: {e}")))
    } else {
        cache
            .get()
            .await
            .map_err(|e| cot::Error::internal(format!("failed to read store: {e}")))
    }
}

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
    head: RequestHead,
    AppraiserExtractor(appraiser): AppraiserExtractor,
    FeedCacheExtractor(cache): FeedCacheExtractor,
) -> cot::Result<Html> {
    info!("serving home");

    let is_admin = appraiser.is_some();
    let feed = get_feed(&cache, &head).await?;

    let entries: Vec<HomeEntry> = visible_entries(&feed)
        .into_iter()
        .filter_map(|(summary, score, _model_version)| {
            if summary.skeet_id.collection() != "app.bsky.feed.post" {
                return None;
            }
            let did = summary.skeet_id.did();
            let rkey = summary.skeet_id.rkey();
            let manual_image = feed.image_appraisals.get(&summary.image_id).map(|a| a.band);
            let manual_skeet = feed.skeet_appraisals.get(&summary.skeet_id).map(|a| a.band);
            let band = image_effective_band(score, manual_image);
            let image_id = summary.image_id.to_string();
            Some(HomeEntry {
                skeet_id_encoded: urlencoding::encode(&summary.skeet_id.to_string()).into_owned(),
                image_id_encoded: urlencoding::encode(&image_id).into_owned(),
                image_id,
                score: format!("{score}"),
                band: band.to_string(),
                manual_skeet_band: manual_skeet.map(|b| b.to_string()).unwrap_or_default(),
                manual_image_band: manual_image.map(|b| b.to_string()).unwrap_or_default(),
                web_url: format!("https://bsky.app/profile/{did}/post/{rkey}"),
            })
        })
        .collect();

    info!(count = entries.len(), is_admin, "serving home entries");
    let rendered = HomeTemplate { entries, is_admin, band_options: band_options() }.render()?;
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
    if let Some(ims) = head.headers.get("if-modified-since").and_then(|v| v.to_str().ok())
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

    let mut response = Response::new(Body::fixed(buf.into_inner()));
    response
        .headers_mut()
        .insert("content-type", "image/png".parse().expect("valid header"));
    response
        .headers_mut()
        .insert("last-modified", last_modified.parse().expect("valid header"));
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

    #[test]
    fn wants_no_cache_true_when_header_present() {
        let req = cot::http::Request::builder()
            .header("cache-control", "no-cache")
            .body(())
            .expect("build");
        let (head, _) = req.into_parts();
        assert!(wants_no_cache(&head));
    }

    #[test]
    fn wants_no_cache_false_when_header_absent() {
        let req = cot::http::Request::builder().body(()).expect("build");
        let (head, _) = req.into_parts();
        assert!(!wants_no_cache(&head));
    }

    #[test]
    fn wants_no_cache_false_for_other_directives() {
        let req = cot::http::Request::builder()
            .header("cache-control", "max-age=60")
            .body(())
            .expect("build");
        let (head, _) = req.into_parts();
        assert!(!wants_no_cache(&head));
    }
}
