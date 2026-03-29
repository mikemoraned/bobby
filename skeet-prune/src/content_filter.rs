use serde_json::Value;

/// Paths within a `getPostThread` JSON response that may contain blocking labels.
const LABEL_PATHS: &[&str] = &[
    "/thread/post/labels",
    "/thread/post/author/labels",
    "/thread/post/embed/record/record/author/labels",
];

/// Check whether a `getPostThread` JSON response contains labels that should
/// cause the post to be blocked. Returns the list of blocked label values found.
pub fn blocked_labels(post_thread_json: &Value) -> Vec<String> {
    let mut found = Vec::new();

    for path in LABEL_PATHS {
        let Some(labels) = post_thread_json.pointer(path).and_then(Value::as_array) else {
            continue;
        };

        for label in labels {
            if let Some(val) = label.get("val").and_then(Value::as_str)
                && shared::labels::EXCLUDED_VALUES.contains(&val)
                && !found.contains(&val.to_string())
            {
                found.push(val.to_string());
            }
        }
    }

    found
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
}
