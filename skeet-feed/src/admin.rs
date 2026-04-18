use std::collections::HashMap;

use cot::html::Html;
use cot::http::request::Parts as RequestHead;
use cot::request::extractors::UrlQuery;
use cot::response::{IntoResponse, Redirect, Response};
use cot::{Body, Template};
use serde::Deserialize;
use std::sync::Arc;

use shared::{Appraiser, Band};
use skeet_store::{Appraisal, DiscoveredAt, ImageId, Score, SkeetId, StoredImageSummary};
use crate::Store;
use crate::effective_band::image_effective_band;
use tracing::{info, instrument};

use crate::AppraiserExtractor;
use crate::handlers::{BandOption, band_options};

const PAGE_SIZE: usize = 10;

pub struct AdminRow {
    pub image_id: String,
    pub row_id: String,
    pub item_id: String,
    pub item_id_encoded: String,
    pub score: String,
    pub auto_band: String,
    pub manual_band: Option<Band>,
    pub manual_band_str: String,
    pub manual_appraiser: String,
    pub effective_band: String,
    pub appraise_kind: String,
    pub web_url: String,
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate<'a> {
    view: &'a str,
    content: &'a str,
    skeet_appraisal_count: usize,
    image_appraisal_count: usize,
}

#[derive(Template)]
#[template(path = "admin_page.html")]
struct AdminPageTemplate<'a> {
    rows: &'a [AdminRow],
    is_first_page: bool,
    view: &'a str,
    next_cursor: Option<&'a str>,
    next_cursor_str: &'a str,
    band_options: Vec<BandOption>,
}

#[derive(Deserialize)]
pub struct AdminQuery {
    pub view: Option<String>,
    pub cursor: Option<String>,
}

#[instrument(skip_all)]
pub async fn admin(
    head: RequestHead,
    AppraiserExtractor(appraiser): AppraiserExtractor,
    Store(store): Store,
    UrlQuery(query): UrlQuery<AdminQuery>,
) -> cot::Result<Response> {
    // Admin guard: redirect unauthenticated users to login
    if appraiser.is_none() {
        let path = head
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/admin");
        let return_to = urlencoding::encode(path);
        return Redirect::new(format!("/auth/login?return_to={return_to}")).into_response();
    }

    let view = query.view.as_deref().unwrap_or("skeet");
    let is_htmx = query.cursor.is_some();

    let before = query
        .cursor
        .as_deref()
        .and_then(|c| c.parse::<i64>().ok())
        .map(|us| DiscoveredAt::new(chrono::DateTime::from_timestamp_micros(us).expect("valid timestamp")));

    let (summaries, next_cursor) = store
        .list_summaries_page(before, PAGE_SIZE)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to list summaries: {e}")))?;

    let image_id_strings: Vec<String> = summaries.iter().map(|s| s.image_id.to_string()).collect();
    let image_ids: Vec<&str> = image_id_strings.iter().map(|s| s.as_str()).collect();
    let score_map = store
        .list_scores_for_ids(&image_ids)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read scores: {e}")))?;

    let skeet_appraisals: HashMap<SkeetId, Appraisal> = store
        .list_all_skeet_appraisals()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read skeet appraisals: {e}")))?
        .into_iter()
        .collect();

    let image_appraisals: HashMap<ImageId, Appraisal> = store
        .list_all_image_appraisals()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read image appraisals: {e}")))?
        .into_iter()
        .collect();

    let rows = build_rows(
        &summaries,
        &score_map,
        &skeet_appraisals,
        &image_appraisals,
        view,
    );

    let next_cursor_str = next_cursor
        .as_ref()
        .map(|c| c.timestamp_micros().to_string())
        .unwrap_or_default();

    let page_html = AdminPageTemplate {
        rows: &rows,
        is_first_page: !is_htmx,
        view,
        next_cursor: if next_cursor.is_some() {
            Some(&next_cursor_str)
        } else {
            None
        },
        next_cursor_str: &next_cursor_str,
        band_options: band_options(),
    }
    .render()?;

    if is_htmx {
        return Html::new(page_html).into_response();
    }

    let rendered = AdminTemplate {
        view,
        content: &page_html,
        skeet_appraisal_count: skeet_appraisals.len(),
        image_appraisal_count: image_appraisals.len(),
    }
    .render()?;

    info!(view, count = rows.len(), "serving admin page");
    Html::new(rendered).into_response()
}

fn build_rows(
    summaries: &[StoredImageSummary],
    score_map: &HashMap<ImageId, Score>,
    skeet_appraisals: &HashMap<SkeetId, Appraisal>,
    image_appraisals: &HashMap<ImageId, Appraisal>,
    view: &str,
) -> Vec<AdminRow> {
    summaries
        .iter()
        .map(|s| {
            let score = score_map.get(&s.image_id).copied();
            let score_str = score
                .map(|sc| format!("{sc}"))
                .unwrap_or_else(|| "—".to_string());
            let auto_band = score
                .map(Band::from_score)
                .map(|b| b.to_string())
                .unwrap_or_else(|| "—".to_string());

            let (manual_appraisal, item_id, appraise_kind) = if view == "image" {
                (
                    image_appraisals.get(&s.image_id),
                    s.image_id.to_string(),
                    "image".to_string(),
                )
            } else {
                (
                    skeet_appraisals.get(&s.skeet_id),
                    s.skeet_id.to_string(),
                    "skeet".to_string(),
                )
            };

            let effective = match (score, manual_appraisal) {
                (Some(sc), Some(appr)) if view == "image" => {
                    image_effective_band(sc, Some(appr.band)).to_string()
                }
                (Some(sc), None) if view == "image" => {
                    image_effective_band(sc, None).to_string()
                }
                (_, Some(appr)) => appr.band.to_string(),
                (Some(sc), None) => Band::from_score(sc).to_string(),
                (None, None) => "—".to_string(),
            };

            let row_id = s.image_id.to_string().replace(':', "-");
            let item_id_encoded = urlencoding::encode(&item_id).into_owned();

            let did = s.skeet_id.did();
            let rkey = s.skeet_id.rkey();
            let web_url = format!("https://bsky.app/profile/{did}/post/{rkey}");

            AdminRow {
                image_id: s.image_id.to_string(),
                row_id,
                item_id,
                item_id_encoded,
                score: score_str,
                auto_band,
                manual_band: manual_appraisal.map(|a| a.band),
                manual_band_str: manual_appraisal
                    .map(|a| a.band.to_string())
                    .unwrap_or_default(),
                manual_appraiser: manual_appraisal
                    .map(|a| a.appraiser.to_string())
                    .unwrap_or_default(),
                effective_band: effective,
                appraise_kind,
                web_url,
            }
        })
        .collect()
}

#[derive(Deserialize)]
pub struct AppraiseQuery {
    pub band: String,
    pub id: String,
}

#[derive(Template)]
#[template(path = "admin_row.html")]
struct AdminRowTemplate<'a> {
    row: &'a AdminRow,
    band_options: Vec<BandOption>,
}

#[instrument(skip_all)]
pub async fn appraise_skeet(
    Store(store): Store,
    AppraiserExtractor(appraiser): AppraiserExtractor,
    UrlQuery(query): UrlQuery<AppraiseQuery>,
) -> cot::Result<Response> {
    let skeet_id: SkeetId = query
        .id
        .parse()
        .map_err(|e| cot::Error::internal(format!("invalid skeet_id: {e}")))?;
    apply_appraisal(&store, appraiser, &query.band, AppraiseTarget::Skeet(&skeet_id)).await?;
    render_updated_row(&store, &skeet_id.to_string(), "skeet").await
}

#[instrument(skip_all)]
pub async fn appraise_image(
    Store(store): Store,
    AppraiserExtractor(appraiser): AppraiserExtractor,
    UrlQuery(query): UrlQuery<AppraiseQuery>,
) -> cot::Result<Response> {
    let image_id: ImageId = query
        .id
        .parse()
        .map_err(|e| cot::Error::internal(format!("invalid image_id: {e}")))?;
    apply_appraisal(&store, appraiser, &query.band, AppraiseTarget::Image(&image_id)).await?;
    render_updated_row(&store, &image_id.to_string(), "image").await
}

enum AppraiseTarget<'a> {
    Skeet(&'a SkeetId),
    Image(&'a ImageId),
}

async fn apply_appraisal(
    store: &skeet_store::SkeetStore,
    appraiser: Option<Arc<Appraiser>>,
    band_str: &str,
    target: AppraiseTarget<'_>,
) -> cot::Result<()> {
    let appraiser = appraiser
        .ok_or_else(|| cot::Error::internal("no appraiser configured — use --local-admin or authenticate"))?;

    if band_str == "clear" {
        match &target {
            AppraiseTarget::Skeet(id) => {
                store.clear_skeet_band(id).await
                    .map_err(|e| cot::Error::internal(format!("failed to clear band: {e}")))?;
                info!(%id, "cleared skeet band");
            }
            AppraiseTarget::Image(id) => {
                store.clear_image_band(id).await
                    .map_err(|e| cot::Error::internal(format!("failed to clear band: {e}")))?;
                info!(%id, "cleared image band");
            }
        }
    } else {
        let band: Band = band_str
            .parse()
            .map_err(|e| cot::Error::internal(format!("invalid band: {e}")))?;
        match &target {
            AppraiseTarget::Skeet(id) => {
                store.set_skeet_band(id, band, &appraiser).await
                    .map_err(|e| cot::Error::internal(format!("failed to set band: {e}")))?;
                info!(%id, %band, "set skeet band");
            }
            AppraiseTarget::Image(id) => {
                store.set_image_band(id, band, &appraiser).await
                    .map_err(|e| cot::Error::internal(format!("failed to set band: {e}")))?;
                info!(%id, %band, "set image band");
            }
        }
    }
    Ok(())
}

async fn render_updated_row(
    store: &skeet_store::SkeetStore,
    id_str: &str,
    view: &str,
) -> cot::Result<Response> {
    // Re-fetch data to render the updated row.
    // For simplicity, fetch all appraisals (they're tiny tables).
    let skeet_appraisals: HashMap<SkeetId, Appraisal> = store
        .list_all_skeet_appraisals()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read appraisals: {e}")))?
        .into_iter()
        .collect();

    let image_appraisals: HashMap<ImageId, Appraisal> = store
        .list_all_image_appraisals()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read appraisals: {e}")))?
        .into_iter()
        .collect();

    // Find the summary that matches. Try by image_id first, then skeet_id.
    let summaries = store
        .list_all_summaries()
        .await
        .map_err(|e| cot::Error::internal(format!("failed to list summaries: {e}")))?;

    let summary = summaries
        .iter()
        .find(|s| {
            if view == "image" {
                s.image_id.to_string() == id_str
            } else {
                s.skeet_id.to_string() == id_str
            }
        })
        .ok_or_else(|| cot::Error::internal(format!("item not found: {id_str}")))?;

    let image_id_strings = [summary.image_id.to_string()];
    let image_ids: Vec<&str> = image_id_strings.iter().map(|s| s.as_str()).collect();
    let score_map = store
        .list_scores_for_ids(&image_ids)
        .await
        .map_err(|e| cot::Error::internal(format!("failed to read scores: {e}")))?;

    let rows = build_rows(
        std::slice::from_ref(summary),
        &score_map,
        &skeet_appraisals,
        &image_appraisals,
        view,
    );

    let row = rows.first().ok_or_else(|| cot::Error::internal("no row built"))?;
    let html = AdminRowTemplate { row, band_options: band_options() }.render()?;

    let mut response = Response::new(Body::fixed(html));
    response
        .headers_mut()
        .insert("content-type", "text/html".parse().expect("valid header"));
    Ok(response)
}
