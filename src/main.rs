use std::fmt;

use futures_util::StreamExt;
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};
use url::Url;

const JETSTREAM_URL: &str = "wss://jetstream2.us-east.bsky.network/subscribe";
const WANTED_COLLECTION: &str = "app.bsky.feed.post";

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
enum BobbyError {
    #[error("invalid Jetstream URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("failed to parse message JSON: {0}")]
    InvalidMessageJson(#[from] serde_json::Error),

    #[error("WebSocket connection failed: {0}")]
    WebSocketConnection(#[from] Box<tokio_tungstenite::tungstenite::Error>),
}

// ---------------------------------------------------------------------------
// Newtypes
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Clone, Deserialize)]
struct Did(String);

impl fmt::Display for Did {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
struct Collection(String);

impl fmt::Display for Collection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, PartialEq, Clone, Deserialize)]
struct Rkey(String);

// ---------------------------------------------------------------------------
// Jetstream event types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JetstreamEvent {
    did: Did,
    #[allow(dead_code)]
    time_us: u64,
    kind: EventKind,
    commit: Option<Commit>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum EventKind {
    Commit,
    Identity,
    Account,
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventKind::Commit => write!(f, "commit"),
            EventKind::Identity => write!(f, "identity"),
            EventKind::Account => write!(f, "account"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Commit {
    operation: Operation,
    collection: Collection,
    #[allow(dead_code)]
    rkey: Rkey,
    record: Option<serde_json::Value>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    Create,
    Update,
    Delete,
}

// ---------------------------------------------------------------------------
// Record / embed types (for image detection)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FeedPost {
    embed: Option<Embed>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "$type")]
enum Embed {
    #[serde(rename = "app.bsky.embed.images")]
    Images(ImageEmbed),
    #[serde(rename = "app.bsky.embed.recordWithMedia")]
    RecordWithMedia(RecordWithMediaEmbed),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ImageEmbed {
    images: Vec<ImageRef>,
}

#[derive(Debug, Deserialize)]
struct RecordWithMediaEmbed {
    media: Option<Box<Embed>>,
}

#[derive(Debug, Deserialize)]
struct ImageRef {
    #[allow(dead_code)]
    alt: Option<String>,
}

// ---------------------------------------------------------------------------
// Image detection
// ---------------------------------------------------------------------------

fn has_images(event: &JetstreamEvent) -> bool {
    if event.kind != EventKind::Commit {
        return false;
    }
    let commit = match &event.commit {
        Some(c) => c,
        None => return false,
    };
    if commit.operation != Operation::Create {
        return false;
    }
    if commit.collection.0 != "app.bsky.feed.post" {
        return false;
    }
    let record = match &commit.record {
        Some(r) => r,
        None => return false,
    };
    let post: FeedPost = match serde_json::from_value(record.clone()) {
        Ok(p) => p,
        Err(_) => return false,
    };
    match &post.embed {
        Some(Embed::Images(_)) => true,
        Some(Embed::RecordWithMedia(rwm)) => matches!(&rwm.media, Some(m) if matches!(**m, Embed::Images(_))),
        _ => false,
    }
}

fn image_count(event: &JetstreamEvent) -> usize {
    let record = event
        .commit
        .as_ref()
        .and_then(|c| c.record.as_ref());
    let record = match record {
        Some(r) => r,
        None => return 0,
    };
    let post: FeedPost = match serde_json::from_value(record.clone()) {
        Ok(p) => p,
        Err(_) => return 0,
    };
    match &post.embed {
        Some(Embed::Images(img)) => img.images.len(),
        Some(Embed::RecordWithMedia(rwm)) => match &rwm.media {
            Some(m) => match m.as_ref() {
                Embed::Images(img) => img.images.len(),
                _ => 0,
            },
            None => 0,
        },
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// URL helper
// ---------------------------------------------------------------------------

fn jetstream_url(base: &str, collection: &str) -> Result<Url, BobbyError> {
    let mut url = Url::parse(base)?;
    url.query_pairs_mut()
        .append_pair("wantedCollections", collection);
    Ok(url)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(operation: &str, collection: &str, record: Option<serde_json::Value>) -> JetstreamEvent {
        JetstreamEvent {
            did: Did("did:plc:test123".to_string()),
            time_us: 1700000000000000,
            kind: EventKind::Commit,
            commit: Some(Commit {
                operation: match operation {
                    "create" => Operation::Create,
                    "update" => Operation::Update,
                    "delete" => Operation::Delete,
                    _ => panic!("unknown operation: {operation}"),
                },
                collection: Collection(collection.to_string()),
                rkey: Rkey("abc123".to_string()),
                record,
            }),
        }
    }

    #[test]
    fn has_images_true_for_image_embed() {
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "check out this photo",
            "embed": {
                "$type": "app.bsky.embed.images",
                "images": [
                    {
                        "alt": "a cat",
                        "image": {
                            "$type": "blob",
                            "ref": {"$link": "bafkrei1234"},
                            "mimeType": "image/jpeg",
                            "size": 12345
                        }
                    }
                ]
            }
        });
        let event = make_event("create", "app.bsky.feed.post", Some(record));
        assert!(has_images(&event));
        assert_eq!(image_count(&event), 1);
    }

    #[test]
    fn has_images_true_for_record_with_media() {
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "quote post with image",
            "embed": {
                "$type": "app.bsky.embed.recordWithMedia",
                "media": {
                    "$type": "app.bsky.embed.images",
                    "images": [
                        {"alt": "pic1"},
                        {"alt": "pic2"}
                    ]
                },
                "record": {
                    "record": {
                        "uri": "at://did:plc:someone/app.bsky.feed.post/xyz",
                        "cid": "bafyrei5678"
                    }
                }
            }
        });
        let event = make_event("create", "app.bsky.feed.post", Some(record));
        assert!(has_images(&event));
        assert_eq!(image_count(&event), 2);
    }

    #[test]
    fn has_images_false_for_text_only_post() {
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "just a text post"
        });
        let event = make_event("create", "app.bsky.feed.post", Some(record));
        assert!(!has_images(&event));
    }

    #[test]
    fn has_images_false_for_external_embed() {
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "check this link",
            "embed": {
                "$type": "app.bsky.embed.external",
                "external": {
                    "uri": "https://example.com",
                    "title": "Example",
                    "description": "An example site"
                }
            }
        });
        let event = make_event("create", "app.bsky.feed.post", Some(record));
        assert!(!has_images(&event));
    }

    #[test]
    fn has_images_false_for_delete_operation() {
        let event = make_event("delete", "app.bsky.feed.post", None);
        assert!(!has_images(&event));
    }

    #[test]
    fn has_images_false_for_non_commit_event() {
        let event = JetstreamEvent {
            did: Did("did:plc:test123".to_string()),
            time_us: 1700000000000000,
            kind: EventKind::Identity,
            commit: None,
        };
        assert!(!has_images(&event));
    }

    #[test]
    fn deserializes_real_jetstream_message() {
        let json = r#"{
            "did": "did:plc:abc123",
            "time_us": 1700000000000000,
            "kind": "commit",
            "commit": {
                "rev": "3abc",
                "operation": "create",
                "collection": "app.bsky.feed.post",
                "rkey": "3abc123",
                "record": {
                    "$type": "app.bsky.feed.post",
                    "text": "hello world",
                    "createdAt": "2024-01-01T00:00:00Z",
                    "embed": {
                        "$type": "app.bsky.embed.images",
                        "images": [
                            {
                                "alt": "",
                                "image": {
                                    "$type": "blob",
                                    "ref": {"$link": "bafkrei1234"},
                                    "mimeType": "image/jpeg",
                                    "size": 50000
                                }
                            }
                        ]
                    }
                },
                "cid": "bafyrei5678"
            }
        }"#;

        let event: JetstreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.did, Did("did:plc:abc123".to_string()));
        assert_eq!(event.kind, EventKind::Commit);
        assert!(event.commit.is_some());
        let commit = event.commit.as_ref().unwrap();
        assert_eq!(commit.operation, Operation::Create);
        assert_eq!(commit.collection, Collection("app.bsky.feed.post".to_string()));
        assert!(has_images(&event));
    }

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
