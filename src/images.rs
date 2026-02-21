use serde::Deserialize;

use crate::jetstream::{EventKind, JetstreamEvent, Operation};

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

pub fn has_images(event: &JetstreamEvent) -> bool {
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
        Some(Embed::RecordWithMedia(rwm)) => {
            matches!(&rwm.media, Some(m) if matches!(**m, Embed::Images(_)))
        }
        _ => false,
    }
}

pub fn image_count(event: &JetstreamEvent) -> usize {
    let record = event.commit.as_ref().and_then(|c| c.record.as_ref());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jetstream::*;

    fn make_event(
        operation: &str,
        collection: &str,
        record: Option<serde_json::Value>,
    ) -> JetstreamEvent {
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
    fn deserializes_commit_with_image_embed() {
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
        assert!(has_images(&event));
        assert_eq!(image_count(&event), 1);
    }
}
