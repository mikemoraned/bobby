mod candidates;
mod db;
mod faces;
mod fetch;
mod images;
mod jetstream;
mod landmarks;
mod scoring;
mod skin_filter;
mod text_filter;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::candidates::{save_candidate, CandidateId};
use crate::db::CandidateDb;
use crate::faces::FaceDetector;
use crate::fetch::fetch_image;
use crate::images::{extract_image_refs, ImagePost};
use crate::jetstream::JetstreamEvent;
use crate::landmarks::LandmarkDetector;
use crate::scoring::score_candidate;
use crate::text_filter::TextDetector;

const JETSTREAM_URL: &str = "wss://jetstream2.us-east.bsky.network/subscribe";
const WANTED_COLLECTION: &str = "app.bsky.feed.post";
const CANDIDATES_DIR: &str = "candidates";
const IMAGE_CHANNEL_CAPACITY: usize = 8;

#[derive(Debug, thiserror::Error)]
enum BobbyError {
    #[error("invalid Jetstream URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("failed to parse message JSON: {0}")]
    InvalidMessageJson(#[from] serde_json::Error),

    #[error("WebSocket connection failed: {0}")]
    WebSocketConnection(#[from] Box<tokio_tungstenite::tungstenite::Error>),
}

fn jetstream_url(base: &str, collection: &str) -> Result<Url, BobbyError> {
    let mut url = Url::parse(base)?;
    url.query_pairs_mut()
        .append_pair("wantedCollections", collection);
    Ok(url)
}

struct DetectionJob {
    did: String,
    rkey: String,
    time_us: u64,
    image_index: usize,
    url: String,
    image: image::DynamicImage,
}

async fn fetch_images_for_post(
    client: &reqwest::Client,
    post: &ImagePost,
) -> Vec<fetch::FetchedImage> {
    let mut results = Vec::new();
    for blob_ref in &post.images {
        match fetch_image(client, &post.did, &blob_ref.cid).await {
            Ok(f) => results.push(f),
            Err(e) => {
                warn!(error = %e, did = %post.did, cid = %blob_ref.cid, "failed to fetch image");
            }
        }
    }
    results
}

fn detection_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn face_detection_thread(
    thread_id: usize,
    rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<DetectionJob>>>,
) {
    info!(thread_id, "loading face detection model");
    let face_detector = FaceDetector::new();
    info!(thread_id, "face detection model loaded");

    info!(thread_id, "loading landmark classification model");
    let landmark_detector = LandmarkDetector::new();
    info!(thread_id, "landmark classification model loaded");

    info!(thread_id, "loading text detection model");
    let text_detector = TextDetector::new();
    info!(thread_id, "text detection model loaded");

    let db_path = std::path::Path::new(CANDIDATES_DIR).join("candidates.db");
    let db = match CandidateDb::new(&db_path) {
        Ok(db) => {
            info!(thread_id, path = %db_path.display(), "opened candidates database");
            db
        }
        Err(e) => {
            error!(thread_id, error = %e, "failed to open candidates database");
            return;
        }
    };

    loop {
        let job = {
            let rx = rx.lock().expect("detection channel mutex poisoned");
            rx.recv()
        };
        let Ok(job) = job else { break };
        let detection = face_detector.detect(&job.image);
        if detection.faces.is_empty() {
            debug!(did = job.did, url = job.url, "no faces detected");
            continue;
        }

        let id = CandidateId::new(&job.did, &job.rkey, job.image_index);
        info!(
            did = job.did,
            url = job.url,
            face_count = detection.faces.len(),
            image_size = format!("{}x{}", detection.image_width, detection.image_height),
            candidate_id = %id,
            "face(s) detected"
        );
        for (i, face) in detection.faces.iter().enumerate() {
            info!(
                face = i,
                confidence = format!("{:.2}", face.confidence),
                bbox = format!("[{:.0},{:.0},{:.0},{:.0}]", face.x1, face.y1, face.x2, face.y2),
                "  face details"
            );
        }

        // Run landmark classification — only save if both face AND landmark detected
        let scene = landmark_detector.classify(&job.image);
        info!(
            candidate_id = %id,
            scene_category = scene.category,
            scene_confidence = format!("{:.3}", scene.confidence),
            is_landmark = scene.is_landmark,
            "scene classification"
        );

        if !scene.is_landmark {
            debug!(candidate_id = %id, "no landmark detected, skipping");
            continue;
        }

        if text_detector.is_mostly_text(&job.image) {
            debug!(candidate_id = %id, "image is mostly text, skipping");
            continue;
        }

        if skin_filter::is_excessive_skin(&job.image) {
            debug!(candidate_id = %id, "image has excessive skin, skipping");
            continue;
        }

        let score = score_candidate(
            &detection.faces,
            detection.image_width,
            detection.image_height,
            scene.confidence,
        );

        info!(
            candidate_id = %id,
            score_overall = format!("{:.3}", score.overall),
            score_face_position = score.face_position,
            score_overlap = format!("{:.3}", score.overlap),
            score_avg_certainty = format!("{:.3}", score.avg_certainty),
            "scored candidate"
        );

        match save_candidate(&job.image, &detection.faces, scene.is_landmark, &id) {
            Ok(saved) => {
                info!(
                    candidate_id = %saved.id,
                    original = %saved.original_path.display(),
                    annotated = %saved.annotated_path.display(),
                    "saved candidate (face + landmark)"
                );

                let discovered_at = now_iso8601();
                if let Err(e) = db.insert(
                    &saved.id.to_string(),
                    &discovered_at,
                    job.time_us,
                    &saved.original_path.to_string_lossy(),
                    &saved.annotated_path.to_string_lossy(),
                    &score,
                ) {
                    warn!(error = %e, candidate_id = %id, "failed to insert into database");
                }
            }
            Err(e) => {
                warn!(error = %e, candidate_id = %id, "failed to save candidate");
            }
        }
    }
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

async fn face_detection_worker(mut rx: mpsc::Receiver<ImagePost>) {
    let client = reqwest::Client::new();

    let num_threads = detection_thread_count();
    let (detect_tx, detect_rx) = std::sync::mpsc::sync_channel::<DetectionJob>(num_threads * 2);
    let detect_rx = std::sync::Arc::new(std::sync::Mutex::new(detect_rx));

    for i in 0..num_threads {
        let rx = detect_rx.clone();
        std::thread::spawn(move || face_detection_thread(i, rx));
    }
    info!(num_threads, "spawned detection threads");

    while let Some(post) = rx.recv().await {
        let did = post.did.to_string();
        let rkey = post.rkey.0.clone();
        let time_us = post.time_us;
        let fetched = fetch_images_for_post(&client, &post).await;
        for (image_index, fetched_image) in fetched.into_iter().enumerate() {
            let job = DetectionJob {
                did: did.clone(),
                rkey: rkey.clone(),
                time_us,
                image_index,
                url: fetched_image.url.to_string(),
                image: fetched_image.image,
            };
            if detect_tx.try_send(job).is_err() {
                debug!("detection thread busy, dropping image");
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), BobbyError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let url = jetstream_url(JETSTREAM_URL, WANTED_COLLECTION)?;

    info!(url = %url, "connecting to Jetstream");

    let (ws_stream, _response) = connect_async(url.as_str())
        .await
        .map_err(|e| BobbyError::WebSocketConnection(Box::new(e)))?;

    info!("connected; listening for messages (Ctrl+C to stop)");

    let (_write, mut read) = ws_stream.split();

    let (tx, rx) = mpsc::channel::<ImagePost>(IMAGE_CHANNEL_CAPACITY);
    tokio::spawn(face_detection_worker(rx));

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<JetstreamEvent>(&text) {
                            Ok(event) => {
                                if let Some(image_post) = extract_image_refs(&event) {
                                    let count = image_post.images.len();
                                    info!(
                                        kind = %event.kind,
                                        did = %event.did,
                                        images = count,
                                        "image post → sending for face detection"
                                    );
                                    if tx.try_send(image_post).is_err() {
                                        debug!("face detection channel full, dropping image post");
                                    }
                                } else {
                                    debug!(
                                        kind = %event.kind,
                                        did = %event.did,
                                        "received message"
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "failed to parse message");
                            }
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        warn!("received unexpected binary message");
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        info!(frame = ?frame, "server closed connection");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        error!(error = %e, "WebSocket error");
                        break;
                    }
                    None => {
                        info!("stream ended");
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received Ctrl+C, shutting down");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jetstream_url_includes_collection_filter() {
        let url = jetstream_url(JETSTREAM_URL, "app.bsky.feed.post").unwrap();
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![(
                std::borrow::Cow::Borrowed("wantedCollections"),
                std::borrow::Cow::Borrowed("app.bsky.feed.post")
            )]
        );
        assert!(url.as_str().starts_with("wss://"));
    }

    #[test]
    fn jetstream_url_returns_error_for_invalid_base() {
        let result = jetstream_url("not a url", "app.bsky.feed.post");
        assert!(result.is_err());
    }
}
