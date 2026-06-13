use serde_json::Value;
use shared::skeet_id::SkeetId;

const BSKY_PUBLIC_API: &str = "https://public.api.bsky.app/xrpc";

/// Paths within a `getPostThread` JSON response that may carry moderation labels.
const LABEL_PATHS: &[&str] = &[
    "/thread/post/labels",
    "/thread/post/author/labels",
    "/thread/post/embed/record/record/author/labels",
];

#[derive(Debug, thiserror::Error)]
pub enum BlueskyError {
    #[error("getPostThread request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("getPostThread returned {status}: {body}")]
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
}

/// Fetch the `app.bsky.feed.getPostThread` JSON for `skeet_id` from the public
/// (unauthenticated) Bluesky AppView, with no surrounding context (`depth=0`,
/// `parentHeight=0`).
pub async fn fetch_post_thread(
    http: &reqwest::Client,
    skeet_id: &SkeetId,
) -> Result<Value, BlueskyError> {
    let uri = skeet_id.to_string();
    let resp = http
        .get(format!("{BSKY_PUBLIC_API}/app.bsky.feed.getPostThread"))
        .query(&[("uri", uri.as_str()), ("depth", "0"), ("parentHeight", "0")])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(BlueskyError::Status { status, body });
    }

    Ok(resp.json().await?)
}

/// The excluding moderation labels on a `getPostThread` response.
///
/// Inspects the post, its author, and a quoted record's author for
/// [`shared::labels::EXCLUDED_VALUES`]. Empty when nothing excluded is present.
pub fn blocked_labels(post_thread_json: &Value) -> Vec<String> {
    let mut found = Vec::new();

    for path in LABEL_PATHS {
        let Some(labels) = post_thread_json.pointer(path).and_then(Value::as_array) else {
            continue;
        };

        for label in labels {
            if let Some(val) = label.get("val").and_then(Value::as_str)
                && shared::labels::EXCLUDED_VALUES.contains(&val)
            {
                let s = val.to_string();
                if !found.contains(&s) {
                    found.push(s);
                }
            }
        }
    }

    found
}

/// Whether a fetched post thread refers to a real, viewable post.
///
/// The public AppView returns a `notFoundPost` node for a deleted post and a
/// `blockedPost` node for one hidden from unauthenticated viewers (both with a
/// `200`); either means the post is no longer viewable. Anything else is treated
/// as an ordinary, available post.
pub fn post_is_available(post_thread_json: &Value) -> bool {
    !matches!(
        post_thread_json
            .pointer("/thread/$type")
            .and_then(Value::as_str),
        Some("app.bsky.feed.defs#notFoundPost" | "app.bsky.feed.defs#blockedPost")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_porn_label() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "labels": [
                        { "val": "porn", "src": "did:plc:someone" }
                    ]
                }
            }
        });
        assert_eq!(blocked_labels(&json), vec!["porn"]);
    }

    #[test]
    fn detects_author_no_unauthenticated() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "author": {
                        "labels": [
                            { "val": "!no-unauthenticated", "src": "did:plc:someone" }
                        ]
                    },
                    "labels": []
                }
            }
        });
        assert_eq!(blocked_labels(&json), vec!["!no-unauthenticated"]);
    }

    #[test]
    fn no_labels_returns_empty() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "author": { "labels": [] },
                    "labels": []
                }
            }
        });
        assert!(blocked_labels(&json).is_empty());
    }

    #[test]
    fn detects_quoted_record_author_no_unauthenticated() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "author": { "labels": [] },
                    "labels": [],
                    "embed": {
                        "record": {
                            "record": {
                                "author": {
                                    "labels": [
                                        { "val": "!no-unauthenticated", "src": "did:plc:quoted" }
                                    ]
                                }
                            }
                        }
                    }
                }
            }
        });
        assert_eq!(blocked_labels(&json), vec!["!no-unauthenticated"]);
    }

    #[test]
    fn ignores_non_blocked_labels() {
        let json = serde_json::json!({
            "thread": {
                "post": {
                    "labels": [
                        { "val": "spam", "src": "did:plc:someone" }
                    ]
                }
            }
        });
        assert!(blocked_labels(&json).is_empty());
    }

    #[test]
    fn available_for_an_ordinary_post() {
        let json = serde_json::json!({
            "thread": {
                "$type": "app.bsky.feed.defs#threadViewPost",
                "post": { "uri": "at://did:plc:abc/app.bsky.feed.post/x" }
            }
        });
        assert!(post_is_available(&json));
    }

    #[test]
    fn unavailable_for_not_found_post() {
        let json = serde_json::json!({
            "thread": { "$type": "app.bsky.feed.defs#notFoundPost", "notFound": true }
        });
        assert!(!post_is_available(&json));
    }

    #[test]
    fn unavailable_for_blocked_post() {
        let json = serde_json::json!({
            "thread": { "$type": "app.bsky.feed.defs#blockedPost", "blocked": true }
        });
        assert!(!post_is_available(&json));
    }
}
