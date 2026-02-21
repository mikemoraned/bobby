use std::fmt;

use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};
use url::Url;

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

#[derive(Debug, PartialEq, Clone)]
struct MessageKind(String);

impl fmt::Display for MessageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq, Clone)]
struct Did(String);

impl fmt::Display for Did {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq)]
struct MessageSummary {
    kind: MessageKind,
    did: Did,
}

fn jetstream_url(base: &str, collection: &str) -> Result<Url, BobbyError> {
    let mut url = Url::parse(base)?;
    url.query_pairs_mut()
        .append_pair("wantedCollections", collection);
    Ok(url)
}

fn parse_message(text: &str) -> Result<MessageSummary, BobbyError> {
    let json: serde_json::Value = serde_json::from_str(text)?;
    Ok(MessageSummary {
        kind: MessageKind(
            json.get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        ),
        did: Did(
            json.get("did")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
        ),
    })
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
                        match parse_message(&text) {
                            Ok(summary) => {
                                info!(kind = %summary.kind, did = %summary.did, "received message");
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

    #[test]
    fn parse_message_extracts_kind_and_did() {
        let json = r#"{"kind":"commit","did":"did:plc:abc123","commit":{}}"#;
        let summary = parse_message(json).unwrap();
        assert_eq!(
            summary,
            MessageSummary {
                kind: MessageKind("commit".to_string()),
                did: Did("did:plc:abc123".to_string()),
            }
        );
    }

    #[test]
    fn parse_message_defaults_missing_fields() {
        let json = r#"{"other":"value"}"#;
        let summary = parse_message(json).unwrap();
        assert_eq!(summary.kind, MessageKind("unknown".to_string()));
        assert_eq!(summary.did, Did("unknown".to_string()));
    }

    #[test]
    fn parse_message_returns_error_for_invalid_json() {
        let result = parse_message("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_message_handles_any_valid_json_object() {
        let json = r#"{"kind":"identity","did":"did:web:example.com","extra":"ignored"}"#;
        let summary = parse_message(json).unwrap();
        assert_eq!(summary.kind, MessageKind("identity".to_string()));
        assert_eq!(summary.did, Did("did:web:example.com".to_string()));
    }
}
