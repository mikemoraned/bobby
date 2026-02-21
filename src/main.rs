mod images;
mod jetstream;

use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::images::{has_images, image_count};
use crate::jetstream::JetstreamEvent;

const JETSTREAM_URL: &str = "wss://jetstream2.us-east.bsky.network/subscribe";
const WANTED_COLLECTION: &str = "app.bsky.feed.post";

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

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<JetstreamEvent>(&text) {
                            Ok(event) => {
                                if has_images(&event) {
                                    let count = image_count(&event);
                                    info!(
                                        kind = %event.kind,
                                        did = %event.did,
                                        images = count,
                                        "image post"
                                    );
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
