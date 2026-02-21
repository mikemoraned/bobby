use std::fmt;

use serde::Deserialize;

use crate::jetstream::{Did, EventKind, JetstreamEvent, Operation, Rkey};

#[derive(Debug, Clone, PartialEq)]
pub struct Cid(pub String);

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone)]
pub struct BlobRef {
    pub cid: Cid,
    #[allow(dead_code)]
    pub mime_type: String,
}

#[derive(Debug)]
pub struct ImagePost {
    pub did: Did,
    #[allow(dead_code)]
    pub rkey: Rkey,
    pub images: Vec<BlobRef>,
}

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
struct BlobObject {
    #[serde(rename = "ref")]
    ref_link: LinkRef,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

#[derive(Debug, Deserialize)]
struct LinkRef {
    #[serde(rename = "$link")]
    link: String,
}

#[derive(Debug, Deserialize)]
struct ImageRef {
    #[allow(dead_code)]
    alt: Option<String>,
    image: Option<BlobObject>,
}

fn extract_blob_refs(embed: &Embed) -> Vec<BlobRef> {
    match embed {
        Embed::Images(img) => img
            .images
            .iter()
            .filter_map(|r| {
                r.image.as_ref().map(|blob| BlobRef {
                    cid: Cid(blob.ref_link.link.clone()),
                    mime_type: blob.mime_type.clone(),
                })
            })
            .collect(),
        Embed::RecordWithMedia(rwm) => match &rwm.media {
            Some(m) => extract_blob_refs(m),
            None => vec![],
        },
        Embed::Other => vec![],
    }
}

pub fn extract_image_refs(event: &JetstreamEvent) -> Option<ImagePost> {
    if event.kind != EventKind::Commit {
        return None;
    }
    let commit = event.commit.as_ref()?;
    if commit.operation != Operation::Create {
        return None;
    }
    if commit.collection.0 != "app.bsky.feed.post" {
        return None;
    }
    let record = commit.record.as_ref()?;
    let post: FeedPost = serde_json::from_value(record.clone()).ok()?;
    let embed = post.embed.as_ref()?;
    let images = extract_blob_refs(embed);
    if images.is_empty() {
        return None;
    }
    Some(ImagePost {
        did: event.did.clone(),
        rkey: commit.rkey.clone(),
        images,
    })
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
    fn extract_image_refs_returns_cids_for_image_embed() {
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
        let post = extract_image_refs(&event).unwrap();
        assert_eq!(post.images.len(), 1);
        assert_eq!(post.images[0].cid, Cid("bafkrei1234".to_string()));
        assert_eq!(post.images[0].mime_type, "image/jpeg");
        assert_eq!(post.did, Did("did:plc:test123".to_string()));
    }

    #[test]
    fn extract_image_refs_returns_none_for_text_only() {
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "just text"
        });
        let event = make_event("create", "app.bsky.feed.post", Some(record));
        assert!(extract_image_refs(&event).is_none());
    }

    #[test]
    fn extract_image_refs_returns_cids_for_record_with_media() {
        let record = serde_json::json!({
            "$type": "app.bsky.feed.post",
            "text": "quote post with image",
            "embed": {
                "$type": "app.bsky.embed.recordWithMedia",
                "media": {
                    "$type": "app.bsky.embed.images",
                    "images": [
                        {
                            "alt": "pic1",
                            "image": {
                                "$type": "blob",
                                "ref": {"$link": "bafkrei_a"},
                                "mimeType": "image/png",
                                "size": 1000
                            }
                        },
                        {
                            "alt": "pic2",
                            "image": {
                                "$type": "blob",
                                "ref": {"$link": "bafkrei_b"},
                                "mimeType": "image/jpeg",
                                "size": 2000
                            }
                        }
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
        let post = extract_image_refs(&event).unwrap();
        assert_eq!(post.images.len(), 2);
        assert_eq!(post.images[0].cid, Cid("bafkrei_a".to_string()));
        assert_eq!(post.images[1].cid, Cid("bafkrei_b".to_string()));
    }

    #[test]
    fn extract_image_refs_returns_none_for_external_embed() {
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
        assert!(extract_image_refs(&event).is_none());
    }

    #[test]
    fn extract_image_refs_returns_none_for_delete_operation() {
        let event = make_event("delete", "app.bsky.feed.post", None);
        assert!(extract_image_refs(&event).is_none());
    }

    #[test]
    fn extract_image_refs_returns_none_for_non_commit_event() {
        let event = JetstreamEvent {
            did: Did("did:plc:test123".to_string()),
            time_us: 1700000000000000,
            kind: EventKind::Identity,
            commit: None,
        };
        assert!(extract_image_refs(&event).is_none());
    }

    #[test]
    fn extract_image_refs_from_full_json() {
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
        let post = extract_image_refs(&event).unwrap();
        assert_eq!(post.images.len(), 1);
        assert_eq!(post.images[0].cid, Cid("bafkrei1234".to_string()));
    }
}
