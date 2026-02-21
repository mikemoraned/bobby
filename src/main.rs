mod faces;
mod fetch;
mod images;
mod jetstream;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::faces::FaceDetector;
use crate::fetch::fetch_image;
use crate::images::{extract_image_refs, ImagePost};
use crate::jetstream::JetstreamEvent;

const JETSTREAM_URL: &str = "wss://jetstream2.us-east.bsky.network/subscribe";
const WANTED_COLLECTION: &str = "app.bsky.feed.post";
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

// Fetches images for a post, returning (blob_index, url, bytes) for each successful fetch.
async fn fetch_images_for_post(
    client: &reqwest::Client,
    post: &ImagePost,
) -> Vec<(fetch::ImageUrl, Vec<u8>)> {
    let mut results = Vec::new();
    for blob_ref in &post.images {
        match fetch_image(client, &post.did, &blob_ref.cid).await {
            Ok(f) => results.push((f.url, f.bytes)),
            Err(e) => {
                warn!(error = %e, did = %post.did, cid = %blob_ref.cid, "failed to fetch image");
            }
        }
    }
    results
}

// Runs face detection on a dedicated thread that owns the detector.
// Receives (did, url, image_bytes) over a channel and logs results.
fn face_detection_thread(rx: std::sync::mpsc::Receiver<(String, String, Vec<u8>)>) {
    info!("loading face detection model");
    let detector = FaceDetector::new();
    info!("face detection model loaded");

    while let Ok((did, url, image_bytes)) = rx.recv() {
        match detector.detect(&image_bytes) {
            Ok(detection) if !detection.faces.is_empty() => {
                info!(
                    did = did,
                    url = url,
                    face_count = detection.faces.len(),
                    image_size = format!("{}x{}", detection.image_width, detection.image_height),
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
            }
            Ok(_) => {
                debug!(did = did, url = url, "no faces detected");
            }
            Err(e) => {
                warn!(error = %e, url = url, "face detection failed");
            }
        }
    }
}

async fn face_detection_worker(mut rx: mpsc::Receiver<ImagePost>) {
    let client = reqwest::Client::new();

    // Use a std channel to send work to the blocking detection thread.
    let (detect_tx, detect_rx) = std::sync::mpsc::sync_channel::<(String, String, Vec<u8>)>(4);
    std::thread::spawn(move || face_detection_thread(detect_rx));

    while let Some(post) = rx.recv().await {
        let fetched = fetch_images_for_post(&client, &post).await;
        for (url, bytes) in fetched {
            if detect_tx
                .try_send((post.did.to_string(), url.to_string(), bytes))
                .is_err()
            {
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
